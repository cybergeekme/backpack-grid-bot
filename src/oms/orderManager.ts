import { Logger } from '../logger';
import { OrderReconciliation, type ReconciliationResult } from './reconciliation';
import type { ExchangeAdapter, Order, OrderRequest, Position } from '../types';

export interface SyncGridResult {
  orders: Order[];
  reconciliation: ReconciliationResult;
  placed: Order[];
  cancelled: Order[];
}

export class OrderManager {
  private readonly logger = new Logger('OrderManager');
  private readonly workingOrders = new Map<string, Order>();
  private readonly reconciliation = new OrderReconciliation();
  private lastKnownPosition?: Position;
  private lastReadOnlyLogKey?: string;
  private lastReadOnlyLogTs = 0;

  constructor(private readonly adapter: ExchangeAdapter) {}

  async syncGrid(symbol: string, desired: OrderRequest[]): Promise<SyncGridResult> {
    const existing = await this.adapter.getOpenOrders(symbol);
    if (this.adapter.getTradingAccessMode() === 'read-only') {
      this.seedWorkingOrders(existing);
      const reconciliation = this.reconciliation.compare(existing, desired);
      this.logReadOnlySkip(symbol, existing.length, desired.length, reconciliation.missingOnExchange.length, reconciliation.unexpectedOnExchange.length);
      return { orders: existing, reconciliation, placed: [], cancelled: [] };
    }
    this.seedWorkingOrders(existing);
    const desiredKeys = new Set(desired.map((o) => this.key(o.side, o.price, o.qty)));
    const cancelled: Order[] = [];

    for (const order of existing) {
      const key = this.key(order.side, order.price, order.qty);
      if (!desiredKeys.has(key)) {
        await this.adapter.cancelOrder(symbol, order.orderId);
        cancelled.push({ ...order, status: 'cancelled' });
      }
    }

    const refreshed = await this.adapter.getOpenOrders(symbol);
    this.seedWorkingOrders(refreshed);
    const existingKeys = new Set(refreshed.map((o) => this.key(o.side, o.price, o.qty)));
    const placed: Order[] = [];

    for (const request of desired) {
      const key = this.key(request.side, request.price, request.qty);
      if (existingKeys.has(key)) continue;
      const order = await this.adapter.placeOrder(request);
      placed.push(order);
      this.applyOrderUpdate(order);
    }

    const finalOrders = await this.adapter.getOpenOrders(symbol);
    this.seedWorkingOrders(finalOrders);
    const reconciliation = this.reconciliation.compare(finalOrders, desired);
    this.logger.info('Grid sync complete.', {
      existing: existing.length,
      desired: desired.length,
      open: finalOrders.length,
      placed: placed.length,
      missingOnExchange: reconciliation.missingOnExchange.length,
      unexpectedOnExchange: reconciliation.unexpectedOnExchange.length
    });
    return { orders: finalOrders, reconciliation, placed, cancelled };
  }

  applyOrderUpdate(order: Order): void {
    if (order.status === 'open' || order.status === 'new') {
      this.workingOrders.set(order.orderId, { ...order });
      return;
    }
    this.workingOrders.delete(order.orderId);
  }

  applyPositionUpdate(position: Position): void {
    this.lastKnownPosition = { ...position };
  }

  getWorkingOrders(): Order[] {
    return [...this.workingOrders.values()];
  }

  getLastKnownPosition(): Position | undefined {
    return this.lastKnownPosition ? { ...this.lastKnownPosition } : undefined;
  }

  private seedWorkingOrders(orders: Order[]): void {
    this.workingOrders.clear();
    for (const order of orders) {
      if (order.status === 'open' || order.status === 'new') {
        this.workingOrders.set(order.orderId, { ...order });
      }
    }
  }

  private logReadOnlySkip(symbol: string, existing: number, desired: number, missingOnExchange: number, unexpectedOnExchange: number): void {
    const now = Date.now();
    const key = `${symbol}:${existing}:${desired}:${missingOnExchange}:${unexpectedOnExchange}`;
    if (key === this.lastReadOnlyLogKey && now - this.lastReadOnlyLogTs < 300_000) {
      return;
    }
    this.lastReadOnlyLogKey = key;
    this.lastReadOnlyLogTs = now;
    this.logger.warn('Read-only adapter mode: skipping grid mutations.', {
      symbol,
      existing,
      desired,
      missingOnExchange,
      unexpectedOnExchange
    });
  }

  private key(side: string, price: number | undefined, qty: number): string {
    return `${side}:${price ?? 'mkt'}:${qty}`;
  }
}
