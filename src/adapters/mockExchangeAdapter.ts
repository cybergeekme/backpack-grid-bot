import { Logger } from '../logger';
import type { AdapterEventListener, Balance, CheckpointCapableAdapter, ExchangeAdapter, Fill, Order, OrderRequest, Position } from '../types';

interface MockExchangeCheckpoint {
  currentPrice: number;
  symbol: string;
  openOrders: Order[];
  recentFills: Fill[];
  position: Position;
  balance: Balance;
  nextId: number;
}

export class MockExchangeAdapter implements ExchangeAdapter, CheckpointCapableAdapter {
  readonly name = 'mock-exchange';
  private readonly logger = new Logger('MockExchangeAdapter');
  private readonly orders = new Map<string, Order>();
  private readonly fills: Fill[] = [];
  private readonly listeners = new Set<AdapterEventListener>();
  private position: Position;
  private balance: Balance;
  private nextId = 1;

  constructor(private currentPrice: number, private readonly symbol: string, initialCash = 100_000) {
    this.position = { symbol, size: 0, entryPrice: 0, unrealizedPnl: 0 };
    this.balance = { asset: 'USDC', total: initialCash, available: initialCash };
  }

  getTradingAccessMode() {
    return 'mock' as const;
  }

  onEvent(listener: AdapterEventListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  async connect(): Promise<void> {
    this.logger.info('Connected to mock exchange.', { symbol: this.symbol, price: this.currentPrice });
    this.emit({ kind: 'connection', state: 'connected' });
  }

  async disconnect(): Promise<void> {
    this.logger.info('Disconnected from mock exchange.');
    this.emit({ kind: 'connection', state: 'disconnected' });
  }

  async getBalance(asset: string): Promise<Balance> {
    if (asset !== this.balance.asset) {
      return { asset, total: 0, available: 0 };
    }
    return { ...this.balance };
  }

  async getOpenOrders(symbol: string): Promise<Order[]> {
    return [...this.orders.values()].filter((o) => o.symbol === symbol && o.status === 'open');
  }

  async getPosition(symbol: string): Promise<Position> {
    if (symbol !== this.symbol) {
      return { symbol, size: 0, entryPrice: 0, unrealizedPnl: 0 };
    }
    const unrealizedPnl = this.position.size * (this.currentPrice - this.position.entryPrice);
    return { ...this.position, unrealizedPnl };
  }

  async placeOrder(request: OrderRequest): Promise<Order> {
    const order: Order = {
      orderId: `MOCK-${this.nextId++}`,
      clientOrderId: request.clientOrderId,
      symbol: request.symbol,
      side: request.side,
      type: request.type,
      status: 'open',
      price: request.price,
      qty: request.qty,
      filledQty: 0,
      reduceOnly: request.reduceOnly ?? false,
      postOnly: request.postOnly ?? false,
      ts: Date.now()
    };
    this.orders.set(order.orderId, order);
    this.logger.info('Placed order.', { orderId: order.orderId, side: order.side, price: order.price, qty: order.qty });
    this.emit({ kind: 'order', order: { ...order } });
    this.tryFill(order);
    return { ...order };
  }

  async cancelOrder(symbol: string, orderId: string): Promise<void> {
    const order = this.orders.get(orderId);
    if (!order || order.symbol !== symbol || order.status !== 'open') return;
    order.status = 'cancelled';
    this.orders.set(orderId, order);
    this.logger.info('Cancelled order.', { orderId });
    this.emit({ kind: 'order', order: { ...order } });
  }

  async markPrice(symbol: string): Promise<number> {
    if (symbol !== this.symbol) {
      throw new Error(`Unknown symbol ${symbol}`);
    }
    return this.currentPrice;
  }

  movePrice(nextPrice: number): void {
    this.currentPrice = nextPrice;
    for (const order of this.orders.values()) {
      this.tryFill(order);
    }
  }

  getFills(): Fill[] {
    return [...this.fills];
  }

  exportCheckpoint(): MockExchangeCheckpoint {
    return {
      currentPrice: this.currentPrice,
      symbol: this.symbol,
      openOrders: [...this.orders.values()]
        .filter((order) => order.status === 'open')
        .map((order) => ({ ...order })),
      recentFills: this.getFills().slice(-20),
      position: { ...this.position },
      balance: { ...this.balance },
      nextId: this.nextId
    };
  }

  importCheckpoint(payload: unknown): void {
    const checkpoint = payload as MockExchangeCheckpoint;
    if (!checkpoint || checkpoint.symbol !== this.symbol) return;
    this.currentPrice = checkpoint.currentPrice;
    this.orders.clear();
    for (const order of checkpoint.openOrders ?? []) {
      this.orders.set(order.orderId, { ...order });
    }
    this.fills.splice(0, this.fills.length, ...((checkpoint.recentFills ?? []).map((fill) => ({ ...fill }))));
    this.position = { ...checkpoint.position };
    this.balance = { ...checkpoint.balance };
    this.nextId = checkpoint.nextId;
    this.logger.info('Restored mock exchange checkpoint.', {
      symbol: this.symbol,
      openOrders: [...this.orders.values()].filter((order) => order.status === 'open').length,
      fills: this.fills.length,
      position: this.position.size,
      currentPrice: this.currentPrice
    });
  }

  private tryFill(order: Order): void {
    if (order.status !== 'open' || order.price === undefined) return;
    const shouldFill = order.side === 'buy' ? this.currentPrice <= order.price : this.currentPrice >= order.price;
    if (!shouldFill) return;

    order.status = 'filled';
    order.filledQty = order.qty;
    this.orders.set(order.orderId, order);

    const signedQty = order.side === 'buy' ? order.qty : -order.qty;
    const newSize = this.position.size + signedQty;
    const fillPrice = order.price;
    if (this.position.size === 0 || Math.sign(this.position.size) === Math.sign(newSize)) {
      const notionalBefore = this.position.entryPrice * Math.abs(this.position.size);
      const notionalDelta = fillPrice * Math.abs(signedQty);
      const totalAbs = Math.abs(this.position.size) + Math.abs(signedQty);
      this.position.entryPrice = totalAbs === 0 ? 0 : (notionalBefore + notionalDelta) / totalAbs;
    } else if (newSize === 0) {
      this.position.entryPrice = 0;
    }
    this.position.size = newSize;

    const fee = fillPrice * order.qty * 0.0002;
    this.balance.total -= fee;
    this.balance.available -= fee;
    const fill: Fill = {
      orderId: order.orderId,
      clientOrderId: order.clientOrderId,
      symbol: order.symbol,
      side: order.side,
      price: fillPrice,
      qty: order.qty,
      fee,
      ts: Date.now()
    };
    this.fills.push(fill);
    this.logger.info('Filled order.', { orderId: order.orderId, fillPrice, qty: order.qty, position: this.position.size });
    this.emit({ kind: 'order', order: { ...order } });
    this.emit({ kind: 'fill', fill: { ...fill } });
    this.emit({ kind: 'position', position: { ...this.position, unrealizedPnl: this.position.size * (this.currentPrice - this.position.entryPrice) } });
  }

  private emit(event: Parameters<AdapterEventListener>[0]): void {
    for (const listener of this.listeners) listener(event);
  }
}
