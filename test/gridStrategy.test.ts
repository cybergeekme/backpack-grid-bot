import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { loadConfig } from '../src/config';
import { OrderManager } from '../src/oms/orderManager';
import { OrderReconciliation } from '../src/oms/reconciliation';
import { RiskEngine } from '../src/risk/riskEngine';
import { GridTradingService } from '../src/service/gridService';
import { ServiceRunner } from '../src/service/serviceRunner';
import { GridStrategyEngine } from '../src/strategy/gridStrategy';
import type { Position } from '../src/types';
import { MockExchangeAdapter } from '../src/adapters/mockExchangeAdapter';

function withDbPath(name: string): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), `${name}-`));
  return path.join(dir, 'state.sqlite');
}

test('grid strategy builds symmetric buy/sell levels', () => {
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

test('service pauses when price moves outside previous grid range', async () => {
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
