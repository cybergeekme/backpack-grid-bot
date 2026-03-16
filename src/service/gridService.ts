import type { AppConfig } from '../config';
import { Logger } from '../logger';
import { AlertManager } from '../alerts';
import { OrderManager } from '../oms/orderManager';
import { SqlitePersistence } from '../persistence/sqliteStore';
import { RiskEngine } from '../risk/riskEngine';
import { InMemoryStateStore } from '../state/stateStore';
import { GridStrategyEngine } from '../strategy/gridStrategy';
import type {
  AdapterEvent,
  CheckpointCapableAdapter,
  ExchangeAdapter,
  Fill,
  GridLevel,
  Order,
  LeverageCapableAdapter,
  OrderRequest,
  Position,
  ReconciliationEvent,
  RiskEvent,
  RuntimeCheckpoint,
  ServiceRuntimeHealth
} from '../types';

function hasCheckpointSupport(adapter: ExchangeAdapter): adapter is ExchangeAdapter & CheckpointCapableAdapter {
  const candidate = adapter as unknown as CheckpointCapableAdapter;
  return typeof candidate.exportCheckpoint === 'function' && typeof candidate.importCheckpoint === 'function';
}

function hasLeverageValidation(adapter: ExchangeAdapter): adapter is ExchangeAdapter & LeverageCapableAdapter {
  const candidate = adapter as unknown as LeverageCapableAdapter;
  return typeof candidate.validateLeverage === 'function';
}

interface FillAlertMetrics {
  fee: number;
  realizedPnl?: number;
  positionAfter?: number;
  entryPriceAfter?: number;
  closedQty?: number;
  openingFill?: boolean;
}

interface NormalizedFillAlertDetails {
  price?: number;
  qty: number;
  fee: number;
  realizedPnl?: number;
  positionAfter?: number;
  entryPriceAfter?: number;
  closedQty?: number;
  openingFill?: boolean;
}

interface InferredFillContext {
  side: 'buy' | 'sell';
  qty: number;
  price?: number;
  orderId?: string;
  clientOrderId?: string;
}

export class GridTradingService {
  private readonly logger = new Logger('GridTradingService');
  private readonly risk: RiskEngine;
  private readonly strategy: GridStrategyEngine;
  private readonly oms: OrderManager;
  private readonly state = new InMemoryStateStore();
  private readonly persistence: SqlitePersistence;
  private unsubscribeAdapterEvents?: () => void;
  private runtimeHealth: ServiceRuntimeHealth = { pauseRequested: false };

  constructor(
    private readonly config: AppConfig,
    private readonly adapter: ExchangeAdapter,
    private readonly alerts?: AlertManager
  ) {
    this.risk = new RiskEngine(config);
    this.strategy = new GridStrategyEngine(config);
    this.oms = new OrderManager(adapter);
    this.persistence = new SqlitePersistence(config.persistencePath, config.serviceName);
  }

  async start(): Promise<void> {
    this.restorePersistedState();
    this.unsubscribeAdapterEvents = this.adapter.onEvent?.((event) => this.handleAdapterEvent(event));
    await this.adapter.connect();
    await this.validateConfiguredLeverage();
    this.persistRuntimeCheckpoint('service_started');
  }

  async stop(): Promise<void> {
    this.persistRuntimeCheckpoint('service_stopping');
    this.unsubscribeAdapterEvents?.();
    this.unsubscribeAdapterEvents = undefined;
    await this.adapter.disconnect();
    this.persistence.close();
  }

  async rebalance(): Promise<void> {
    const priorSnapshot = this.state.get().snapshot;
    const midPrice = await this.adapter.markPrice(this.config.symbol);
    this.runtimeHealth.lastMidPrice = midPrice;
    this.updatePauseHealth(priorSnapshot, midPrice);
    const snapshot = this.strategy.buildGrid(midPrice);
    if (this.runtimeHealth.pauseRequested) {
      this.persistRuntimeCheckpoint('rebalance_paused_out_of_range');
      this.logger.warn('Skipping order sync because market moved outside configured grid range.', {
        symbol: this.config.symbol,
        midPrice,
        pauseReason: this.runtimeHealth.pauseReason
      });
      this.emitAlert({
        key: `pause:${this.config.symbol}:${this.runtimeHealth.pauseReason ?? 'unknown'}`,
        severity: 'warn',
        title: 'Grid paused: price out of range',
        message: 'Rebalance skipped because mid price moved outside the configured grid buffer.',
        details: {
          symbol: this.config.symbol,
          midPrice,
          pauseReason: this.runtimeHealth.pauseReason
        }
      });
      return;
    }
    this.state.setSnapshot(snapshot);
    this.persistence.persistSnapshot(snapshot);

    const previousPosition = this.state.get().position;
    const previousOrders = this.state.get().workingOrders;
    const position = await this.adapter.getPosition(this.config.symbol);
    this.state.setPosition(position);
    this.persistence.persistPosition(position);
    this.oms.applyPositionUpdate(position);
    const desired: OrderRequest[] = [];
    const activeLevels = this.selectActiveLevels(snapshot.levels, midPrice, position);
    let plannedPosition = this.normalizePositionSize(position.size);

    for (const level of activeLevels) {
      const isShortOnlyReduceBuy = this.config.gridMode === 'short_only' && level.side === 'buy';
      const maxReducibleQty = isShortOnlyReduceBuy ? Math.max(0, -plannedPosition) : undefined;
      const rawQty = isShortOnlyReduceBuy ? Math.min(level.qty, maxReducibleQty ?? 0) : level.qty;
      const qty = this.normalizeOrderQty(rawQty);
      if (!(qty > 0)) continue;

      const projectedPosition: Position = {
        ...position,
        size: this.normalizePositionSize(plannedPosition)
      };
      const risk = this.risk.validateNewOrder(projectedPosition, level.side, qty);
      if (!risk.ok) {
        const riskEvent: RiskEvent = {
          symbol: this.config.symbol,
          side: level.side,
          qty,
          reason: risk.reason ?? 'unknown',
          levelIndex: level.index,
          price: level.price,
          ts: Date.now()
        };
        this.persistence.persistRiskEvent(riskEvent);
        this.logger.warn('Risk rejected level.', { level: level.index, side: level.side, price: level.price, qty, reason: risk.reason, plannedPosition });
        continue;
      }
      desired.push({
        clientOrderId: `grid-${level.side}-${level.index}-${Math.round(level.price * 100)}`,
        symbol: this.config.symbol,
        side: level.side,
        type: 'limit',
        price: level.price,
        qty,
        postOnly: true,
        reduceOnly: isShortOnlyReduceBuy
      });
      plannedPosition = this.normalizePositionSize(plannedPosition + (level.side === 'buy' ? qty : -qty));
    }

    const tradingAccessMode = this.adapter.getTradingAccessMode();
    const syncResult = await this.oms.syncGrid(this.config.symbol, desired);
    this.emitReconcileFillFallback(previousPosition, position, previousOrders, syncResult.orders);
    this.state.setWorkingOrders(syncResult.orders);
    this.persistence.persistOrders(syncResult.orders);
    if (this.config.telegramNotifyOrderEvents) {
      for (const order of syncResult.cancelled) {
        this.emitAlert({
          key: `order:${order.orderId}:cancelled:${Date.now()}`,
          severity: 'info',
          title: 'Order cancelled',
          message: `${order.side.toUpperCase()} ${order.symbol} order was cancelled during grid sync.`,
          details: {
            symbol: order.symbol,
            side: order.side,
            status: 'cancelled',
            price: order.price,
            qty: order.qty,
            filledQty: order.filledQty,
            reduceOnly: order.reduceOnly,
            postOnly: order.postOnly,
            orderId: order.orderId,
            clientOrderId: order.clientOrderId
          },
          dedupMs: 0
        });
      }
      for (const order of syncResult.placed) {
        this.emitAlert({
          key: `order:${order.orderId}:open:${Date.now()}`,
          severity: 'info',
          title: 'Order open',
          message: `${order.side.toUpperCase()} ${order.symbol} order was placed during grid sync.`,
          details: {
            symbol: order.symbol,
            side: order.side,
            status: order.status,
            price: order.price,
            qty: order.qty,
            filledQty: order.filledQty,
            reduceOnly: order.reduceOnly,
            postOnly: order.postOnly,
            orderId: order.orderId,
            clientOrderId: order.clientOrderId
          },
          dedupMs: 0
        });
      }
    }
    const reconciliationEvent: ReconciliationEvent = {
      symbol: this.config.symbol,
      matched: syncResult.reconciliation.matched,
      missingOnExchange: syncResult.reconciliation.missingOnExchange,
      unexpectedOnExchange: syncResult.reconciliation.unexpectedOnExchange,
      ts: Date.now()
    };
    this.persistence.persistReconciliation(reconciliationEvent);
    if (reconciliationEvent.missingOnExchange.length > 0 || reconciliationEvent.unexpectedOnExchange.length > 0) {
      this.emitAlert({
        key: `reconciliation:${this.config.symbol}`,
        severity: 'warn',
        title: 'Order reconciliation mismatch',
        message: 'Exchange open orders diverged from the desired grid set.',
        details: {
          symbol: this.config.symbol,
          matched: reconciliationEvent.matched,
          missingOnExchange: reconciliationEvent.missingOnExchange.length,
          unexpectedOnExchange: reconciliationEvent.unexpectedOnExchange.length
        }
      });
    }
    const balance = await this.adapter.getBalance(this.config.quoteAsset);
    const refreshedPosition = await this.adapter.getPosition(this.config.symbol);
    this.state.setPosition(refreshedPosition);
    this.persistence.persistPosition(refreshedPosition);
    this.oms.applyPositionUpdate(refreshedPosition);
    this.persistRuntimeCheckpoint('rebalance_complete');

    this.logger.info('Rebalanced grid.', {
      symbol: this.config.symbol,
      midPrice,
      totalLevels: snapshot.levels.length,
      activeLevels: activeLevels.length,
      gridMode: this.config.gridMode,
      leverage: this.config.leverage,
      workingOrders: syncResult.orders.length,
      position: refreshedPosition.size,
      balance: balance.available,
      tradingAccessMode
    });
  }

  snapshot() {
    return this.state.get();
  }

  getRuntimeHealth(): ServiceRuntimeHealth {
    return { ...this.runtimeHealth };
  }

  async reconcileWithExchange(): Promise<void> {
    const stateBefore = this.state.get();
    const previousPosition = stateBefore.position;
    const previousOrders = stateBefore.workingOrders;
    const currentSnapshot = stateBefore.snapshot;
    const midPrice = await this.adapter.markPrice(this.config.symbol);
    this.runtimeHealth.lastMidPrice = midPrice;
    this.updatePauseHealth(currentSnapshot, midPrice);
    const orders = await this.adapter.getOpenOrders(this.config.symbol);
    const position = await this.adapter.getPosition(this.config.symbol);
    this.emitReconcileFillFallback(previousPosition, position, previousOrders, orders);
    this.state.setWorkingOrders(orders);
    this.state.setPosition(position);
    this.oms.applyPositionUpdate(position);
    this.persistence.persistOrders(orders);
    this.persistence.persistPosition(position);
    this.runtimeHealth.lastReconciliationTs = Date.now();
    this.persistRuntimeCheckpoint('rest_reconcile');
    this.logger.info('REST reconciliation complete.', {
      symbol: this.config.symbol,
      openOrders: orders.length,
      position: position.size
    });
  }

  private async validateConfiguredLeverage(): Promise<void> {
    if (this.config.leverage <= 1 || !hasLeverageValidation(this.adapter)) return;
    const result = await this.adapter.validateLeverage(this.config.symbol, this.config.leverage);
    if (!result.accepted) {
      throw new Error(result.message);
    }
    this.logger.warn('Configured leverage requires exchange-side confirmation.', {
      symbol: this.config.symbol,
      leverage: this.config.leverage,
      accountLimit: result.accountLimit,
      verified: result.verified,
      canSet: result.canSet,
      message: result.message
    });
  }

  private selectActiveLevels(levels: GridLevel[], midPrice: number, position?: Position): GridLevel[] {
    const sorted = [...levels].sort((a, b) => Math.abs((a.price ?? midPrice) - midPrice) - Math.abs((b.price ?? midPrice) - midPrice));
    const sells = sorted.filter((level) => level.side === 'sell');
    const buys = sorted.filter((level) => level.side === 'buy');
    const perSide = this.config.gridActiveLevels;

    if (this.config.gridMode === 'short_only') {
      const sellLevels = sells.slice(0, perSide);
      const reduceBuySlots = position && position.size < 0
        ? Math.min(buys.length, perSide, Math.ceil(Math.abs(position.size) / this.config.orderSize))
        : 0;
      const reduceBuyLevels = buys.slice(0, reduceBuySlots);
      return [...reduceBuyLevels, ...sellLevels].sort((a, b) => a.price - b.price);
    }

    if (this.config.gridMode === 'short_bias') {
      const sellCount = Math.min(sells.length, Math.max(1, Math.round(perSide * this.config.gridShortBiasSellRatio)));
      const buyCount = Math.min(buys.length, perSide);
      return [...buys.slice(0, buyCount), ...sells.slice(0, sellCount)].sort((a, b) => a.price - b.price);
    }

    return [...buys.slice(0, perSide), ...sells.slice(0, perSide)].sort((a, b) => a.price - b.price);
  }

  private restorePersistedState(): void {
    const recovery = this.persistence.loadLatestRecovery();
    const recovered = recovery.state;
    this.runtimeHealth = recovery.checkpoint?.health ?? { pauseRequested: false };
    this.state.restore(recovered);
    if (recovered.position) {
      this.oms.applyPositionUpdate(recovered.position);
    }
    for (const order of recovered.workingOrders) {
      this.oms.applyOrderUpdate(order);
    }
    if (recovery.checkpoint?.adapter && hasCheckpointSupport(this.adapter)) {
      this.adapter.importCheckpoint(recovery.checkpoint.adapter);
    }
    if (recovered.snapshot || recovered.position || recovered.workingOrders.length || recovered.recentFills.length) {
      this.logger.info('Recovered persisted runtime state.', {
        snapshot: Boolean(recovered.snapshot),
        position: recovered.position?.size,
        workingOrders: recovered.workingOrders.length,
        recentFills: recovered.recentFills.length,
        checkpointTs: recovery.checkpoint?.ts
      });
    }
  }

  private updatePauseHealth(snapshot: { levels: { price: number }[] } | undefined, midPrice: number): void {
    const wasPaused = this.runtimeHealth.pauseRequested;
    const priorReason = this.runtimeHealth.pauseReason;
    if (!snapshot || snapshot.levels.length === 0) {
      this.runtimeHealth.pauseRequested = false;
      this.runtimeHealth.pauseReason = undefined;
      this.runtimeHealth.outOfRangeSinceTs = undefined;
      if (wasPaused) {
        this.emitAlert({
          key: `pause-cleared:${this.config.symbol}`,
          severity: 'info',
          title: 'Grid pause cleared',
          message: 'Service resumed normal grid eligibility after price moved back into range.',
          details: { symbol: this.config.symbol, midPrice, priorReason }
        });
      }
      return;
    }
    const prices = snapshot.levels.map((level) => level.price);
    const minPrice = Math.min(...prices);
    const maxPrice = Math.max(...prices);
    const bufferRatio = this.config.serviceOutOfRangePauseBps / 10_000;
    const lower = minPrice * (1 - bufferRatio);
    const upper = maxPrice * (1 + bufferRatio);
    const outOfRange = midPrice < lower || midPrice > upper;
    if (outOfRange) {
      this.runtimeHealth.pauseRequested = true;
      this.runtimeHealth.pauseReason = 'mid_price_out_of_grid_range';
      this.runtimeHealth.outOfRangeSinceTs ??= Date.now();
      return;
    }
    this.runtimeHealth.pauseRequested = false;
    this.runtimeHealth.pauseReason = undefined;
    this.runtimeHealth.outOfRangeSinceTs = undefined;
    if (wasPaused) {
      this.emitAlert({
        key: `pause-cleared:${this.config.symbol}`,
        severity: 'info',
        title: 'Grid pause cleared',
        message: 'Service resumed normal grid eligibility after price moved back into range.',
        details: { symbol: this.config.symbol, midPrice, priorReason }
      });
    }
  }

  private handleAdapterEvent(event: AdapterEvent): void {
    switch (event.kind) {
      case 'order':
        this.oms.applyOrderUpdate(event.order);
        this.state.setWorkingOrders(this.oms.getWorkingOrders());
        this.persistence.persistOrders(this.oms.getWorkingOrders());
        this.persistRuntimeCheckpoint('adapter_order_update');
        this.logger.info('Processed adapter order update.', {
          orderId: event.order.orderId,
          status: event.order.status,
          filledQty: event.order.filledQty
        });
        const shouldEmitOrderStatusAlert = this.config.telegramNotifyOrderEvents
          && !(event.order.status === 'filled' && this.config.telegramNotifyFills);
        if (shouldEmitOrderStatusAlert) {
          this.emitAlert({
            key: `order:${event.order.orderId}:${event.order.status}:${event.order.ts}`,
            severity: event.order.status === 'rejected' ? 'warn' : 'info',
            title: `Order ${event.order.status}`,
            message: `${event.order.side.toUpperCase()} ${event.order.symbol} order is now ${event.order.status}.`,
            details: {
              symbol: event.order.symbol,
              side: event.order.side,
              status: event.order.status,
              price: event.order.price,
              qty: event.order.qty,
              filledQty: event.order.filledQty,
              reduceOnly: event.order.reduceOnly,
              postOnly: event.order.postOnly,
              orderId: event.order.orderId,
              clientOrderId: event.order.clientOrderId
            },
            dedupMs: 0
          });
        }
        return;
      case 'position':
        this.oms.applyPositionUpdate(event.position);
        this.state.setPosition(event.position);
        this.persistence.persistPosition(event.position);
        this.persistRuntimeCheckpoint('adapter_position_update');
        this.logger.info('Processed adapter position update.', {
          symbol: event.position.symbol,
          size: event.position.size,
          entryPrice: event.position.entryPrice
        });
        return;
      case 'fill': {
        const positionBeforeFill = this.state.get().position;
        const fillMetrics = this.estimateFillAlertMetrics(positionBeforeFill, event.fill);
        this.state.pushFill(event.fill);
        this.persistence.persistFill(event.fill);
        this.persistRuntimeCheckpoint('adapter_fill_update');
        this.logger.info('Processed adapter fill update.', {
          orderId: event.fill.orderId,
          qty: event.fill.qty,
          price: event.fill.price,
          fee: event.fill.fee,
          realizedPnl: fillMetrics.realizedPnl,
          positionAfter: fillMetrics.positionAfter
        });
        if (this.config.telegramNotifyFills) {
          const normalizedDetails = this.normalizeFillAlertDetails(event.fill.price, event.fill.qty, fillMetrics);
          this.emitAlert({
            key: `fill:${event.fill.orderId}:${event.fill.ts}`,
            severity: 'info',
            title: 'Order filled',
            message: `${event.fill.side.toUpperCase()} ${event.fill.symbol} order filled.`,
            details: {
              symbol: event.fill.symbol,
              side: event.fill.side,
              price: normalizedDetails.price,
              qty: normalizedDetails.qty,
              fee: normalizedDetails.fee,
              realizedPnl: normalizedDetails.realizedPnl,
              positionAfter: normalizedDetails.positionAfter,
              entryPriceAfter: normalizedDetails.entryPriceAfter,
              closedQty: normalizedDetails.closedQty,
              openingFill: normalizedDetails.openingFill,
              orderId: event.fill.orderId,
              clientOrderId: event.fill.clientOrderId
            },
            dedupMs: 0
          });
        }
        return;
      }
      case 'connection':
        this.persistence.persistCheckpoint('connection', { ...event, ts: Date.now() });
        this.logger.info('Adapter connection event.', {
          state: event.state,
          message: event.message
        });
        return;
    }
  }

  private persistRuntimeCheckpoint(reason: string): void {
    const state = this.state.get();
    const checkpoint: RuntimeCheckpoint = {
      reason,
      state: {
        snapshot: state.snapshot,
        position: state.position
      },
      adapter: hasCheckpointSupport(this.adapter) ? this.adapter.exportCheckpoint() : undefined,
      health: this.runtimeHealth,
      ts: Date.now()
    };
    this.persistence.persistCheckpoint('runtime', checkpoint);
  }

  private emitReconcileFillFallback(
    previousPosition: Position | undefined,
    currentPosition: Position,
    previousOrders: Order[],
    currentOrders: Order[]
  ): void {
    if (!this.config.telegramNotifyFills) return;
    const inferred = this.inferFillFromReconciliation(previousPosition, currentPosition, previousOrders, currentOrders);
    if (!inferred) return;

    const syntheticFill: Fill = {
      orderId: inferred.orderId ?? `reconcile-${this.config.symbol}-${Date.now()}`,
      clientOrderId: inferred.clientOrderId ?? '',
      symbol: this.config.symbol,
      side: inferred.side,
      price: inferred.price ?? 0,
      qty: inferred.qty,
      fee: 0,
      ts: Date.now()
    };
    const fillMetrics = this.estimateFillAlertMetrics(previousPosition, syntheticFill);
    const normalizedDetails = this.normalizeFillAlertDetails(inferred.price, syntheticFill.qty, {
      ...fillMetrics,
      positionAfter: currentPosition.size,
      entryPriceAfter: currentPosition.entryPrice
    });
    this.logger.info('Emitting reconciliation fill fallback alert.', {
      symbol: this.config.symbol,
      side: inferred.side,
      qty: normalizedDetails.qty,
      price: normalizedDetails.price,
      orderId: inferred.orderId,
      positionBefore: previousPosition?.size,
      positionAfter: normalizedDetails.positionAfter
    });
    this.emitAlert({
      key: `fill-fallback:${syntheticFill.orderId}:${syntheticFill.ts}`,
      severity: 'info',
      title: 'Order filled',
      message: `${syntheticFill.side.toUpperCase()} ${syntheticFill.symbol} order filled (reconciled).`,
      details: {
        symbol: syntheticFill.symbol,
        side: syntheticFill.side,
        price: normalizedDetails.price,
        qty: normalizedDetails.qty,
        fee: normalizedDetails.fee,
        realizedPnl: normalizedDetails.realizedPnl,
        positionAfter: normalizedDetails.positionAfter,
        entryPriceAfter: normalizedDetails.entryPriceAfter,
        closedQty: normalizedDetails.closedQty,
        openingFill: normalizedDetails.openingFill,
        orderId: syntheticFill.orderId,
        clientOrderId: syntheticFill.clientOrderId,
        source: 'reconciliation_fallback'
      },
      dedupMs: 0
    });
  }

  private inferFillFromReconciliation(
    previousPosition: Position | undefined,
    currentPosition: Position,
    previousOrders: Order[],
    currentOrders: Order[]
  ): InferredFillContext | undefined {
    const before = Number(previousPosition?.size ?? 0);
    const after = Number(currentPosition.size ?? 0);
    const delta = after - before;
    if (!delta) return undefined;

    const side: 'buy' | 'sell' = delta > 0 ? 'buy' : 'sell';
    const qty = Math.abs(delta);
    const currentIds = new Set(currentOrders.map((order) => order.orderId));
    const missingCandidates = previousOrders.filter((order) => !currentIds.has(order.orderId) && order.side === side);
    const exactQtyCandidate = missingCandidates.find((order) => Math.abs(order.qty - qty) < 1e-9);
    const candidate = exactQtyCandidate ?? missingCandidates[0];

    return {
      side,
      qty,
      price: candidate?.price,
      orderId: candidate?.orderId,
      clientOrderId: candidate?.clientOrderId
    };
  }

  private orderPrecisionDecimals(): number {
    const normalizedOrderSize = String(this.config.orderSize);
    return normalizedOrderSize.includes('.')
      ? normalizedOrderSize.split('.')[1]?.length ?? 0
      : 0;
  }

  private pricePrecisionDecimals(value?: number): number {
    const normalizedPrice = String(value ?? 0);
    return normalizedPrice.includes('.')
      ? normalizedPrice.split('.')[1]?.length ?? 0
      : 0;
  }

  private normalizeDisplayNumber(value: number | undefined, decimals: number): number | undefined {
    if (value === undefined || !Number.isFinite(value)) return value;
    if (decimals <= 0) return Math.round(value);
    return Number(value.toFixed(decimals));
  }

  private normalizePnlDisplay(value: number | undefined): number | undefined {
    if (value === undefined || !Number.isFinite(value)) return value;
    return Number(value.toFixed(8));
  }

  private normalizeFillAlertDetails(price: number | undefined, qty: number, metrics: FillAlertMetrics): NormalizedFillAlertDetails {
    const qtyDecimals = this.orderPrecisionDecimals();
    const priceDecimals = this.pricePrecisionDecimals(price ?? metrics.entryPriceAfter);
    return {
      price: this.normalizeDisplayNumber(price, priceDecimals),
      qty: this.normalizeDisplayNumber(qty, qtyDecimals) ?? qty,
      fee: this.normalizePnlDisplay(metrics.fee) ?? 0,
      realizedPnl: this.normalizePnlDisplay(metrics.realizedPnl),
      positionAfter: this.normalizeDisplayNumber(metrics.positionAfter, qtyDecimals),
      entryPriceAfter: this.normalizeDisplayNumber(metrics.entryPriceAfter, priceDecimals),
      closedQty: this.normalizeDisplayNumber(metrics.closedQty, qtyDecimals),
      openingFill: metrics.openingFill
    };
  }

  private normalizeOrderQty(qty: number): number {
    const decimals = this.orderPrecisionDecimals();
    if (decimals <= 0) {
      return Math.round(qty);
    }
    return Number(qty.toFixed(decimals));
  }

  private normalizePositionSize(size: number): number {
    const decimals = this.orderPrecisionDecimals();
    const normalized = decimals <= 0 ? Math.round(size) : Number(size.toFixed(decimals));
    const epsilon = decimals <= 0 ? 1e-9 : 10 ** (-(decimals + 3));
    return Math.abs(normalized) < epsilon ? 0 : normalized;
  }

  private estimateFillAlertMetrics(positionBefore: Position | undefined, fill: Fill): FillAlertMetrics {
    const fee = Number(fill.fee ?? 0);
    const beforeSize = Number(positionBefore?.size ?? 0);
    const beforeEntry = Number(positionBefore?.entryPrice ?? 0);
    const signedFillQty = fill.side === 'buy' ? fill.qty : -fill.qty;
    const afterSize = this.normalizePositionSize(beforeSize + signedFillQty);

    if (beforeSize === 0) {
      return {
        fee,
        realizedPnl: 0,
        positionAfter: afterSize,
        entryPriceAfter: fill.price,
        closedQty: 0,
        openingFill: true
      };
    }

    const reducing = Math.sign(beforeSize) !== Math.sign(signedFillQty);
    const closedQty = reducing ? Math.min(Math.abs(beforeSize), Math.abs(signedFillQty)) : 0;
    const realizedPnl = beforeSize > 0
      ? (fill.price - beforeEntry) * closedQty
      : (beforeEntry - fill.price) * closedQty;

    if (!reducing) {
      const totalAbs = Math.abs(beforeSize) + Math.abs(signedFillQty);
      const entryPriceAfter = totalAbs === 0
        ? 0
        : ((beforeEntry * Math.abs(beforeSize)) + (fill.price * Math.abs(signedFillQty))) / totalAbs;
      return {
        fee,
        realizedPnl: 0,
        positionAfter: afterSize,
        entryPriceAfter,
        closedQty: 0,
        openingFill: true
      };
    }

    if (afterSize === 0) {
      return {
        fee,
        realizedPnl,
        positionAfter: 0,
        entryPriceAfter: 0,
        closedQty,
        openingFill: false
      };
    }

    if (Math.sign(afterSize) === Math.sign(beforeSize)) {
      return {
        fee,
        realizedPnl,
        positionAfter: afterSize,
        entryPriceAfter: beforeEntry,
        closedQty,
        openingFill: false
      };
    }

    return {
      fee,
      realizedPnl,
      positionAfter: afterSize,
      entryPriceAfter: fill.price,
      closedQty,
      openingFill: false
    };
  }

  private emitAlert(event: Parameters<AlertManager['notify']>[0]): void {
    if (!this.alerts) return;
    void this.alerts.notify(event);
  }
}
