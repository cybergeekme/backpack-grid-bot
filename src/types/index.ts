export type Side = 'buy' | 'sell';
export type OrderType = 'limit' | 'market';
export type OrderStatus = 'new' | 'open' | 'filled' | 'cancelled' | 'rejected';
export type TradingAccessMode = 'mock' | 'read-only' | 'live';

export interface MarketTick {
  symbol: string;
  price: number;
  ts: number;
}

export interface Position {
  symbol: string;
  size: number;
  entryPrice: number;
  unrealizedPnl: number;
}

export interface OrderRequest {
  clientOrderId: string;
  symbol: string;
  side: Side;
  type: OrderType;
  price?: number;
  qty: number;
  reduceOnly?: boolean;
  postOnly?: boolean;
}

export interface Order {
  orderId: string;
  clientOrderId: string;
  symbol: string;
  side: Side;
  type: OrderType;
  status: OrderStatus;
  price?: number;
  qty: number;
  filledQty: number;
  reduceOnly: boolean;
  postOnly: boolean;
  ts: number;
}

export interface Fill {
  orderId: string;
  clientOrderId: string;
  symbol: string;
  side: Side;
  price: number;
  qty: number;
  fee: number;
  ts: number;
}

export interface Balance {
  asset: string;
  total: number;
  available: number;
}

export interface GridLevel {
  index: number;
  side: Side;
  price: number;
  qty: number;
}

export interface RiskDecision {
  ok: boolean;
  reason?: string;
}

export interface StrategySnapshot {
  symbol: string;
  midPrice: number;
  levels: GridLevel[];
  range?: {
    minPrice: number;
    maxPrice: number;
  };
}

export interface RuntimeState {
  snapshot?: StrategySnapshot;
  workingOrders: Order[];
  position?: Position;
  recentFills: Fill[];
}

export interface ReconciliationEvent {
  symbol: string;
  matched: number;
  missingOnExchange: OrderRequest[];
  unexpectedOnExchange: Order[];
  ts: number;
}

export interface RiskEvent {
  symbol: string;
  side: Side;
  qty: number;
  reason: string;
  levelIndex: number;
  price: number;
  ts: number;
}

export interface RuntimeCheckpointState {
  snapshot?: StrategySnapshot;
  position?: Position;
}

export interface RuntimeCheckpoint {
  reason: string;
  state: RuntimeCheckpointState;
  adapter?: unknown;
  health?: ServiceRuntimeHealth;
  ts: number;
}

export interface ServiceRuntimeHealth {
  pauseRequested: boolean;
  pauseReason?: string;
  outOfRangeSinceTs?: number;
  lastMidPrice?: number;
  lastReconciliationTs?: number;
}

export interface CheckpointCapableAdapter {
  exportCheckpoint(): unknown;
  importCheckpoint(payload: unknown): void;
}

export interface AdapterOrderUpdate {
  kind: 'order';
  order: Order;
  raw?: unknown;
}

export interface AdapterPositionUpdate {
  kind: 'position';
  position: Position;
  raw?: unknown;
}

export interface AdapterFillUpdate {
  kind: 'fill';
  fill: Fill;
  raw?: unknown;
}

export interface AdapterConnectionEvent {
  kind: 'connection';
  state: 'connecting' | 'connected' | 'disconnected' | 'error';
  message?: string;
  raw?: unknown;
}

export type AdapterEvent =
  | AdapterOrderUpdate
  | AdapterPositionUpdate
  | AdapterFillUpdate
  | AdapterConnectionEvent;

export type AdapterEventListener = (event: AdapterEvent) => void;

export interface ExchangeAdapter {
  readonly name: string;
  getTradingAccessMode(): TradingAccessMode;
  connect(): Promise<void>;
  disconnect(): Promise<void>;
  getBalance(asset: string): Promise<Balance>;
  getOpenOrders(symbol: string): Promise<Order[]>;
  getPosition(symbol: string): Promise<Position>;
  placeOrder(request: OrderRequest): Promise<Order>;
  cancelOrder(symbol: string, orderId: string): Promise<void>;
  markPrice(symbol: string): Promise<number>;
  onEvent?(listener: AdapterEventListener): () => void;
}
