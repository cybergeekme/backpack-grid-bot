import type { AppConfig } from '../config';
import { Logger } from '../logger';
import { OrderManager } from '../oms/orderManager';
import { SqlitePersistence } from '../persistence/sqliteStore';
import { RiskEngine } from '../risk/riskEngine';
import { InMemoryStateStore } from '../state/stateStore';
import { GridStrategyEngine } from '../strategy/gridStrategy';
import type {
  AdapterEvent,
  CheckpointCapableAdapter,
  ExchangeAdapter,
  OrderRequest,
  ReconciliationEvent,
  RiskEvent,
  RuntimeCheckpoint
} from '../types';

function hasCheckpointSupport(adapter: ExchangeAdapter): adapter is ExchangeAdapter & CheckpointCapableAdapter {
  const candidate = adapter as unknown as CheckpointCapableAdapter;
  return typeof candidate.exportCheckpoint === 'function' && typeof candidate.importCheckpoint === 'function';
}

export class GridTradingService {
  private readonly logger = new Logger('GridTradingService');
  private readonly risk: RiskEngine;
  private readonly strategy: GridStrategyEngine;
  private readonly oms: OrderManager;
  private readonly state = new InMemoryStateStore();
  private readonly persistence: SqlitePersistence;
  private unsubscribeAdapterEvents?: () => void;

  constructor(
    private readonly config: AppConfig,
    private readonly adapter: ExchangeAdapter
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
    const midPrice = await this.adapter.markPrice(this.config.symbol);
    const snapshot = this.strategy.buildGrid(midPrice);
    this.state.setSnapshot(snapshot);
    this.persistence.persistSnapshot(snapshot);

    const position = await this.adapter.getPosition(this.config.symbol);
    this.state.setPosition(position);
    this.persistence.persistPosition(position);
    this.oms.applyPositionUpdate(position);
    const desired: OrderRequest[] = [];

    for (const level of snapshot.levels) {
      const risk = this.risk.validateNewOrder(position, level.side, level.qty);
      if (!risk.ok) {
        const riskEvent: RiskEvent = {
          symbol: this.config.symbol,
          side: level.side,
          qty: level.qty,
          reason: risk.reason ?? 'unknown',
          levelIndex: level.index,
          price: level.price,
          ts: Date.now()
        };
        this.persistence.persistRiskEvent(riskEvent);
        this.logger.warn('Risk rejected level.', { level: level.index, side: level.side, price: level.price, reason: risk.reason });
        continue;
      }
      desired.push({
        clientOrderId: `grid-${level.side}-${level.index}-${Math.round(level.price * 100)}`,
        symbol: this.config.symbol,
        side: level.side,
        type: 'limit',
        price: level.price,
        qty: level.qty,
        postOnly: true,
        reduceOnly: false
      });
    }

    const syncResult = await this.oms.syncGrid(this.config.symbol, desired);
    this.state.setWorkingOrders(syncResult.orders);
    this.persistence.persistOrders(syncResult.orders);
    const reconciliationEvent: ReconciliationEvent = {
      symbol: this.config.symbol,
      matched: syncResult.reconciliation.matched,
      missingOnExchange: syncResult.reconciliation.missingOnExchange,
      unexpectedOnExchange: syncResult.reconciliation.unexpectedOnExchange,
      ts: Date.now()
    };
    this.persistence.persistReconciliation(reconciliationEvent);
    const balance = await this.adapter.getBalance(this.config.quoteAsset);
    const refreshedPosition = await this.adapter.getPosition(this.config.symbol);
    this.state.setPosition(refreshedPosition);
    this.persistence.persistPosition(refreshedPosition);
    this.oms.applyPositionUpdate(refreshedPosition);
    this.persistRuntimeCheckpoint('rebalance_complete');

    this.logger.info('Rebalanced grid.', {
      symbol: this.config.symbol,
      midPrice,
      workingOrders: syncResult.orders.length,
      position: refreshedPosition.size,
      balance: balance.available
    });
  }

  snapshot() {
    return this.state.get();
  }

  private restorePersistedState(): void {
    const recovery = this.persistence.loadLatestRecovery();
    const recovered = recovery.checkpoint?.state ?? recovery.state;
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
      case 'fill':
        this.state.pushFill(event.fill);
        this.persistence.persistFill(event.fill);
        this.persistRuntimeCheckpoint('adapter_fill_update');
        this.logger.info('Processed adapter fill update.', {
          orderId: event.fill.orderId,
          qty: event.fill.qty,
          price: event.fill.price
        });
        return;
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
    const checkpoint: RuntimeCheckpoint = {
      reason,
      state: this.state.get(),
      adapter: hasCheckpointSupport(this.adapter) ? this.adapter.exportCheckpoint() : undefined,
      ts: Date.now()
    };
    this.persistence.persistCheckpoint('runtime', checkpoint);
  }
}
