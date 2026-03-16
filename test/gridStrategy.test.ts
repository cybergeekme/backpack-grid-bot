import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { AlertManager, type AlertEvent } from '../src/alerts';
import { loadConfig } from '../src/config';
import { OrderManager } from '../src/oms/orderManager';
import { OrderReconciliation } from '../src/oms/reconciliation';
import { RiskEngine } from '../src/risk/riskEngine';
import { GridTradingService } from '../src/service/gridService';
import { ServiceRunner } from '../src/service/serviceRunner';
import { GridStrategyEngine } from '../src/strategy/gridStrategy';
import type { Position } from '../src/types';
import { MockExchangeAdapter } from '../src/adapters/mockExchangeAdapter';
import { BackpackFuturesAdapter } from '../src/adapters/backpackAdapter';

function withDbPath(name: string): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), `${name}-`));
  return path.join(dir, 'state.sqlite');
}

function resetGridEnv(): void {
  delete process.env.GRID_MIN_PRICE;
  delete process.env.GRID_MAX_PRICE;
  delete process.env.GRID_MODE;
  delete process.env.GRID_ACTIVE_LEVELS;
  delete process.env.GRID_SHORT_BIAS_SELL_RATIO;
  delete process.env.GRID_LEVERAGE;
}

test('grid strategy builds symmetric buy/sell levels', () => {
  resetGridEnv();
  process.env.GRID_LEVELS = '2';
  process.env.GRID_SPACING_BPS = '50';
  process.env.GRID_ORDER_SIZE = '0.01';
  const config = loadConfig();
  const engine = new GridStrategyEngine(config);
  const snapshot = engine.buildGrid(100);

  assert.equal(snapshot.levels.length, 4);
  assert.deepEqual(snapshot.levels.map((l) => [l.side, l.price]), [
    ['buy', 99],
    ['buy', 99.5],
    ['sell', 100.5],
    ['sell', 101]
  ]);
});

test('grid strategy builds bounded range levels and classifies them around mid price', () => {
  resetGridEnv();
  process.env.GRID_LEVELS = '8';
  process.env.GRID_ORDER_SIZE = '0.01';
  process.env.GRID_MIN_PRICE = '1500';
  process.env.GRID_MAX_PRICE = '2300';
  const config = loadConfig();
  const engine = new GridStrategyEngine(config);
  const snapshot = engine.buildGrid(1900);

  assert.deepEqual(snapshot.range, { minPrice: 1500, maxPrice: 2300 });
  assert.equal(snapshot.levels.length, 8);
  assert.equal(snapshot.levels.filter((l) => l.side === 'buy').length, 4);
  assert.equal(snapshot.levels.filter((l) => l.side === 'sell').length, 4);
  assert.equal(snapshot.levels[0]?.price, 1588.89);
  assert.equal(snapshot.levels.at(-1)?.price, 2211.11);
});

test('risk engine rejects breaches and kill switch', () => {
  process.env.GRID_MAX_POSITION_ABS = '0.05';
  process.env.GRID_KILL_SWITCH = 'false';
  const config = loadConfig();
  const risk = new RiskEngine(config);
  const position: Position = { symbol: 'BTC-PERP', size: 0.04, entryPrice: 100, unrealizedPnl: 0 };

  assert.deepEqual(risk.validateNewOrder(position, 'buy', 0.02), {
    ok: false,
    reason: 'max_position_breached'
  });

  process.env.GRID_KILL_SWITCH = 'true';
  const blocked = new RiskEngine(loadConfig());
  assert.deepEqual(blocked.validateNewOrder(position, 'sell', 0.01), {
    ok: false,
    reason: 'kill_switch_enabled'
  });
});

test('reconciliation reports missing and unexpected orders', () => {
  const reconciliation = new OrderReconciliation();
  const result = reconciliation.compare(
    [
      {
        orderId: '1',
        clientOrderId: 'a',
        symbol: 'BTC_USDC_PERP',
        side: 'buy',
        type: 'limit',
        status: 'open',
        price: 99,
        qty: 0.01,
        filledQty: 0,
        reduceOnly: false,
        postOnly: true,
        ts: 1
      }
    ],
    [
      {
        clientOrderId: 'target-buy',
        symbol: 'BTC_USDC_PERP',
        side: 'buy',
        type: 'limit',
        price: 99,
        qty: 0.01,
        postOnly: true,
        reduceOnly: false
      },
      {
        clientOrderId: 'target-sell',
        symbol: 'BTC_USDC_PERP',
        side: 'sell',
        type: 'limit',
        price: 101,
        qty: 0.01,
        postOnly: true,
        reduceOnly: false
      }
    ]
  );

  assert.equal(result.matched, 1);
  assert.equal(result.missingOnExchange.length, 1);
  assert.equal(result.unexpectedOnExchange.length, 0);
  assert.equal(result.missingOnExchange[0]?.side, 'sell');
});

test('grid service consumes adapter events and updates state', async () => {
  resetGridEnv();
  process.env.GRID_LEVELS = '1';
  process.env.GRID_SPACING_BPS = '50';
  process.env.GRID_ORDER_SIZE = '0.01';
  process.env.GRID_MAX_POSITION_ABS = '1';
  process.env.GRID_KILL_SWITCH = 'false';
  process.env.GRID_USE_BACKPACK = 'false';
  process.env.GRID_DB_PATH = withDbPath('grid-service');
  process.env.GRID_SERVICE_NAME = 'grid-service-test';

  const config = loadConfig();
  const adapter = new MockExchangeAdapter(100, config.symbol, 10_000);
  const service = new GridTradingService(config, adapter);

  await service.start();
  await service.rebalance();
  adapter.movePrice(99);

  const state = service.snapshot();
  assert.ok(state.position);
  assert.equal(state.position?.size, 0.01);
  assert.ok(state.recentFills.length >= 1);
  assert.ok(state.workingOrders.every((order) => order.status === 'open'));

  await service.stop();
});

test('order manager applies streamed order updates', async () => {
  const adapter = new MockExchangeAdapter(100, 'BTC_USDC_PERP');
  const oms = new OrderManager(adapter);

  const order = await adapter.placeOrder({
    clientOrderId: 'stream-me',
    symbol: 'BTC_USDC_PERP',
    side: 'buy',
    type: 'limit',
    price: 99,
    qty: 0.01,
    postOnly: true,
    reduceOnly: false
  });

  oms.applyOrderUpdate(order);
  assert.equal(oms.getWorkingOrders().length, 1);

  oms.applyOrderUpdate({ ...order, status: 'filled', filledQty: order.qty });
  assert.equal(oms.getWorkingOrders().length, 0);
});

test('service selects a bounded active window with short bias instead of hanging the full range', async () => {
  resetGridEnv();
  process.env.GRID_LEVELS = '100';
  process.env.GRID_ORDER_SIZE = '0.01';
  process.env.GRID_MAX_POSITION_ABS = '1';
  process.env.GRID_KILL_SWITCH = 'false';
  process.env.GRID_USE_BACKPACK = 'false';
  process.env.GRID_DB_PATH = withDbPath('grid-short-bias');
  process.env.GRID_SERVICE_NAME = 'grid-short-bias-test';
  process.env.GRID_MIN_PRICE = '1500';
  process.env.GRID_MAX_PRICE = '2300';
  process.env.GRID_MODE = 'short_bias';
  process.env.GRID_ACTIVE_LEVELS = '5';
  process.env.GRID_SHORT_BIAS_SELL_RATIO = '3';
  process.env.GRID_LEVERAGE = '5';

  const config = loadConfig();
  const adapter = new MockExchangeAdapter(1900, config.symbol, 10_000);
  const service = new GridTradingService(config, adapter);
  await service.start();
  await service.rebalance();

  const state = service.snapshot();
  assert.equal(state.snapshot?.levels.length, 100);
  assert.equal(state.workingOrders.length, 20);
  assert.equal(state.workingOrders.filter((o) => o.side === 'buy').length, 5);
  assert.equal(state.workingOrders.filter((o) => o.side === 'sell').length, 15);
  assert.equal(config.leverage, 5);
  assert.equal(config.gridMode, 'short_bias');

  await service.stop();
});

test('service pauses when price moves outside previous grid range', async () => {
  resetGridEnv();
  process.env.GRID_LEVELS = '1';
  process.env.GRID_SPACING_BPS = '50';
  process.env.GRID_ORDER_SIZE = '0.01';
  process.env.GRID_MAX_POSITION_ABS = '1';
  process.env.GRID_USE_BACKPACK = 'false';
  process.env.GRID_DB_PATH = withDbPath('grid-pause');
  process.env.GRID_SERVICE_NAME = 'grid-pause-test';
  process.env.GRID_SERVICE_OUT_OF_RANGE_BPS = '10';

  const config = loadConfig();
  const adapter = new MockExchangeAdapter(100, config.symbol, 10_000);
  const service = new GridTradingService(config, adapter);
  await service.start();
  await service.rebalance();

  adapter.movePrice(130);
  await service.rebalance();

  assert.equal(service.getRuntimeHealth().pauseRequested, true);
  assert.equal(service.getRuntimeHealth().pauseReason, 'mid_price_out_of_grid_range');
  await service.stop();
});

test('service runner enters SAFE_MODE after consecutive errors', async () => {
  process.env.GRID_LEVELS = '1';
  process.env.GRID_SPACING_BPS = '50';
  process.env.GRID_ORDER_SIZE = '0.01';
  process.env.GRID_MAX_POSITION_ABS = '1';
  process.env.GRID_USE_BACKPACK = 'false';
  process.env.GRID_DB_PATH = withDbPath('grid-safe-mode');
  process.env.GRID_SERVICE_NAME = 'grid-safe-mode-test';
  process.env.GRID_SERVICE_LOOP_MS = '5';
  process.env.GRID_SERVICE_RECONCILE_MS = '5';
  process.env.GRID_SERVICE_ERROR_THRESHOLD = '1';

  const config = loadConfig();
  const adapter = new MockExchangeAdapter(100, config.symbol, 10_000);
  let throwsRemaining = 1;
  const originalMarkPrice = adapter.markPrice.bind(adapter);
  adapter.markPrice = async (symbol: string) => {
    if (throwsRemaining > 0) {
      throwsRemaining -= 1;
      throw new Error('synthetic failure');
    }
    return originalMarkPrice(symbol);
  };

  const service = new GridTradingService(config, adapter);
  const runner = new ServiceRunner(config, service);
  const runPromise = runner.start();
  await new Promise((resolve) => setTimeout(resolve, 40));
  await runner.stop('test_done');
  await runPromise;

  assert.equal(runner.snapshot().state, 'STOPPED');
  assert.equal(runner.snapshot().safeModeReason, 'consecutive_error_threshold:1');
});

test('alert manager deduplicates repeated events and swallows sink failures', async () => {
  const delivered: AlertEvent[] = [];
  const manager = new AlertManager(
    { enabled: true, minSeverity: 'info', dedupMs: 60_000 },
    [
      { notify: async (event) => void delivered.push(event) },
      { notify: async () => { throw new Error('synthetic notifier failure'); } }
    ]
  );

  await manager.notify({ key: 'same-key', severity: 'warn', title: 'A', message: 'first' });
  await manager.notify({ key: 'same-key', severity: 'warn', title: 'A', message: 'second' });
  await manager.notify({ key: 'other-key', severity: 'warn', title: 'B', message: 'third' });

  assert.equal(delivered.length, 2);
  assert.equal(delivered[0]?.message, 'first');
  assert.equal(delivered[1]?.key, 'other-key');
});

test('service emits Telegram alerts for each order status update when enabled', async () => {
  resetGridEnv();
  process.env.GRID_LEVELS = '1';
  process.env.GRID_SPACING_BPS = '50';
  process.env.GRID_ORDER_SIZE = '0.01';
  process.env.GRID_MAX_POSITION_ABS = '1';
  process.env.GRID_USE_BACKPACK = 'false';
  process.env.GRID_DB_PATH = withDbPath('grid-order-alerts');
  process.env.GRID_SERVICE_NAME = 'grid-order-alerts-test';
  process.env.TELEGRAM_ALERTS_ENABLED = 'true';
  process.env.TELEGRAM_ALERT_LEVEL = 'info';
  process.env.TELEGRAM_NOTIFY_ORDER_EVENTS = 'true';

  const delivered: AlertEvent[] = [];
  const alerts = new AlertManager(
    { enabled: true, minSeverity: 'info', dedupMs: 60_000 },
    [{ notify: async (event) => void delivered.push(event) }]
  );

  const config = loadConfig();
  const adapter = new MockExchangeAdapter(100, config.symbol, 10_000);
  const service = new GridTradingService(config, adapter, alerts);
  await service.start();
  await service.rebalance();

  const state = service.snapshot();
  const openOrder = state.workingOrders[0];
  assert.ok(openOrder);

  await adapter.cancelOrder(config.symbol, openOrder.orderId);
  await new Promise((resolve) => setTimeout(resolve, 10));

  const orderAlerts = delivered.filter((event) => event.key.startsWith('order:'));
  assert.ok(orderAlerts.length >= 3);
  assert.ok(orderAlerts.some((event) => event.title === 'Order open'));
  assert.ok(orderAlerts.some((event) => event.title === 'Order cancelled'));
  assert.ok(orderAlerts.every((event) => event.dedupMs === 0));

  await service.stop();
});

test('runtime checkpoints stay compact and recover from persisted orders instead of embedding them', async () => {
  process.env.GRID_LEVELS = '1';
  process.env.GRID_SPACING_BPS = '50';
  process.env.GRID_ORDER_SIZE = '0.01';
  process.env.GRID_MAX_POSITION_ABS = '1';
  process.env.GRID_KILL_SWITCH = 'false';
  process.env.GRID_USE_BACKPACK = 'false';
  process.env.GRID_DB_PATH = withDbPath('grid-runtime-checkpoint');
  process.env.GRID_SERVICE_NAME = 'grid-runtime-checkpoint-test';

  const config = loadConfig();
  const adapter = new MockExchangeAdapter(100, config.symbol, 10_000);
  const service = new GridTradingService(config, adapter);
  await service.start();
  await service.rebalance();

  const { DatabaseSync } = await import('node:sqlite');
  const db = new DatabaseSync(config.persistencePath, { readOnly: true });
  const row = db
    .prepare(
      `SELECT payload FROM service_checkpoints
       WHERE service_name = ? AND checkpoint_kind = 'runtime'
       ORDER BY created_ts DESC, id DESC
       LIMIT 1`
    )
    .get(config.serviceName) as { payload: string };
  db.close();

  const checkpoint = JSON.parse(row.payload);
  assert.equal(Array.isArray(checkpoint.state?.workingOrders), false);
  assert.equal(Array.isArray(checkpoint.state?.recentFills), false);
  assert.ok((row.payload as string).length < 50_000);

  await service.stop();
});

test('service recovers persisted snapshot, fills, and mock adapter checkpoint on restart', async () => {
  process.env.GRID_LEVELS = '1';
  process.env.GRID_SPACING_BPS = '50';
  process.env.GRID_ORDER_SIZE = '0.01';
  process.env.GRID_MAX_POSITION_ABS = '1';
  process.env.GRID_KILL_SWITCH = 'false';
  process.env.GRID_USE_BACKPACK = 'false';
  process.env.GRID_DB_PATH = withDbPath('grid-recovery');
  process.env.GRID_SERVICE_NAME = 'grid-recovery-test';

  const config = loadConfig();
  const adapter1 = new MockExchangeAdapter(100, config.symbol, 10_000);
  const service1 = new GridTradingService(config, adapter1);
  await service1.start();
  await service1.rebalance();
  adapter1.movePrice(99);
  const before = service1.snapshot();
  await service1.stop();

  const adapter2 = new MockExchangeAdapter(100, config.symbol, 10_000);
  const service2 = new GridTradingService(config, adapter2);
  await service2.start();
  const recovered = service2.snapshot();

  assert.equal(recovered.snapshot?.midPrice, before.snapshot?.midPrice);
  assert.equal(recovered.position?.size, before.position?.size);
  assert.equal(recovered.recentFills.length, before.recentFills.length);
  assert.equal((await adapter2.getOpenOrders(config.symbol)).length, before.workingOrders.length);

  await service2.stop();
});


test('backpack adapter exposes read-only mode and blocks mutations while disconnected from live trading', async () => {
  const adapter = new BackpackFuturesAdapter({ enableWebSocket: false, tradingEnabled: false });
  assert.equal(adapter.getTradingAccessMode(), 'read-only');

  await assert.rejects(
    () => adapter.placeOrder({
      clientOrderId: 'readonly-order',
      symbol: 'ETH_USDC_PERP',
      side: 'buy',
      type: 'limit',
      price: 100,
      qty: 0.01,
      postOnly: true,
      reduceOnly: false
    }),
    /not connected/i
  );
});

test('backpack adapter treats missing position as flat instead of fatal', async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (input: string | URL | Request, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
    if (url.includes('/api/v1/time')) {
      return new Response('{"serverTime":1773623800000}', {
        status: 200,
        headers: { 'content-type': 'application/json' }
      });
    }
    if (url.includes('/api/v1/position')) {
      return new Response('{"code":"RESOURCE_NOT_FOUND","message":"Not Found"}', {
        status: 404,
        headers: { 'content-type': 'application/json' }
      });
    }
    throw new Error(`unexpected fetch: ${url}`);
  };

  try {
    const adapter = new BackpackFuturesAdapter({
      enableWebSocket: false,
      tradingEnabled: false,
      apiKey: 'test-api-key',
      apiSecret: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA='
    });
    await adapter.connect();
    const position = await adapter.getPosition('ETH_USDC_PERP');
    assert.deepEqual(position, {
      symbol: 'ETH_USDC_PERP',
      size: 0,
      entryPrice: 0,
      unrealizedPnl: 0
    });
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('backpack adapter prefers collateral equity for trading balance', async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (input: string | URL | Request, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
    if (url.includes('/api/v1/time')) {
      return new Response('{"serverTime":1773623800000}', {
        status: 200,
        headers: { 'content-type': 'application/json' }
      });
    }
    if (url.includes('/api/v1/capital/collateral')) {
      return new Response('{"netEquity":"149.4","netEquityAvailable":"149.4","collateral":[{"symbol":"USDC","totalQuantity":"149.4","availableQuantity":"0","balanceNotional":"149.4"}]}', {
        status: 200,
        headers: { 'content-type': 'application/json' }
      });
    }
    throw new Error(`unexpected fetch: ${url}`);
  };

  try {
    const adapter = new BackpackFuturesAdapter({
      enableWebSocket: false,
      tradingEnabled: false,
      apiKey: 'test-api-key',
      apiSecret: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA='
    });
    await adapter.connect();
    const balance = await adapter.getBalance('USDC');
    assert.deepEqual(balance, {
      asset: 'USDC',
      total: 149.4,
      available: 149.4
    });
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('backpack adapter falls back to capital payload when collateral equity is empty', async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (input: string | URL | Request, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
    if (url.includes('/api/v1/time')) {
      return new Response('{"serverTime":1773623800000}', {
        status: 200,
        headers: { 'content-type': 'application/json' }
      });
    }
    if (url.includes('/api/v1/capital/collateral')) {
      return new Response('{}', {
        status: 200,
        headers: { 'content-type': 'application/json' }
      });
    }
    if (url.includes('/api/v1/capital')) {
      return new Response('{}', {
        status: 200,
        headers: { 'content-type': 'application/json' }
      });
    }
    throw new Error(`unexpected fetch: ${url}`);
  };

  try {
    const adapter = new BackpackFuturesAdapter({
      enableWebSocket: false,
      tradingEnabled: false,
      apiKey: 'test-api-key',
      apiSecret: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA='
    });
    await adapter.connect();
    const balance = await adapter.getBalance('USDC');
    assert.deepEqual(balance, {
      asset: 'USDC',
      total: 0,
      available: 0
    });
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('backpack adapter rejects configured leverage above account limit', async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (input: string | URL | Request, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
    if (url.includes('/api/v1/time')) {
      return new Response('{"serverTime":1773623800000}', { status: 200, headers: { 'content-type': 'application/json' } });
    }
    if (url.includes('/api/v1/account')) {
      return new Response('{"leverageLimit":"3"}', { status: 200, headers: { 'content-type': 'application/json' } });
    }
    throw new Error(`unexpected fetch: ${url}`);
  };

  try {
    const adapter = new BackpackFuturesAdapter({
      enableWebSocket: false,
      tradingEnabled: true,
      apiKey: 'test-api-key',
      apiSecret: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA='
    });
    await adapter.connect();
    const result = await adapter.validateLeverage('ETH_USDC_PERP', 5);
    assert.equal(result.accepted, false);
    assert.equal(result.accountLimit, 3);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('backpack adapter accepts configured leverage within account limit but cannot verify applied leverage', async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (input: string | URL | Request, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
    if (url.includes('/api/v1/time')) {
      return new Response('{"serverTime":1773623800000}', { status: 200, headers: { 'content-type': 'application/json' } });
    }
    if (url.includes('/api/v1/account')) {
      return new Response('{"leverageLimit":"75"}', { status: 200, headers: { 'content-type': 'application/json' } });
    }
    throw new Error(`unexpected fetch: ${url}`);
  };

  try {
    const adapter = new BackpackFuturesAdapter({
      enableWebSocket: false,
      tradingEnabled: true,
      apiKey: 'test-api-key',
      apiSecret: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA='
    });
    await adapter.connect();
    const result = await adapter.validateLeverage('ETH_USDC_PERP', 5);
    assert.equal(result.accepted, true);
    assert.equal(result.verified, false);
    assert.equal(result.canSet, false);
    assert.equal(result.accountLimit, 75);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('order manager skips mutations when adapter is read-only', async () => {
  const adapter = new MockExchangeAdapter(100, 'BTC_USDC_PERP');
  const originalMode = adapter.getTradingAccessMode.bind(adapter);
  adapter.getTradingAccessMode = () => 'read-only';

  let placeCalls = 0;
  let cancelCalls = 0;
  const originalPlace = adapter.placeOrder.bind(adapter);
  const originalCancel = adapter.cancelOrder.bind(adapter);
  adapter.placeOrder = async (request) => {
    placeCalls += 1;
    return originalPlace(request);
  };
  adapter.cancelOrder = async (symbol, orderId) => {
    cancelCalls += 1;
    return originalCancel(symbol, orderId);
  };

  await originalPlace({
    clientOrderId: 'existing-buy',
    symbol: 'BTC_USDC_PERP',
    side: 'buy',
    type: 'limit',
    price: 99,
    qty: 0.01,
    postOnly: true,
    reduceOnly: false
  });
  placeCalls = 0;

  const oms = new OrderManager(adapter);
  const result = await oms.syncGrid('BTC_USDC_PERP', [{
    clientOrderId: 'target-sell',
    symbol: 'BTC_USDC_PERP',
    side: 'sell',
    type: 'limit',
    price: 101,
    qty: 0.01,
    postOnly: true,
    reduceOnly: false
  }]);

  assert.equal(adapter.getTradingAccessMode(), 'read-only');
  assert.equal(placeCalls, 0);
  assert.equal(cancelCalls, 0);
  assert.equal(result.orders.length, 1);
  assert.equal(result.reconciliation.missingOnExchange.length, 1);
  assert.equal(result.reconciliation.unexpectedOnExchange.length, 1);
  adapter.getTradingAccessMode = originalMode;
});


test('service starts and rebalances safely in read-only mode without placing or cancelling orders', async () => {
  process.env.GRID_LEVELS = '1';
  process.env.GRID_SPACING_BPS = '50';
  process.env.GRID_ORDER_SIZE = '0.01';
  process.env.GRID_MAX_POSITION_ABS = '1';
  process.env.GRID_KILL_SWITCH = 'false';
  process.env.GRID_USE_BACKPACK = 'false';
  process.env.GRID_DB_PATH = withDbPath('grid-readonly');
  process.env.GRID_SERVICE_NAME = 'grid-readonly-test';

  const config = loadConfig();
  const adapter = new MockExchangeAdapter(100, config.symbol, 10_000);
  adapter.getTradingAccessMode = () => 'read-only';

  let placeCalls = 0;
  let cancelCalls = 0;
  adapter.placeOrder = async () => {
    placeCalls += 1;
    throw new Error('placeOrder should not be called in read-only mode');
  };
  adapter.cancelOrder = async () => {
    cancelCalls += 1;
    throw new Error('cancelOrder should not be called in read-only mode');
  };

  const service = new GridTradingService(config, adapter);
  await service.start();
  await service.rebalance();
  const state = service.snapshot();

  assert.equal(placeCalls, 0);
  assert.equal(cancelCalls, 0);
  assert.equal(state.snapshot?.midPrice, 100);
  assert.equal(state.position?.size, 0);
  assert.equal(state.workingOrders.length, 0);

  await service.stop();
});
