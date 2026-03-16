import crypto from 'node:crypto';
import { BackpackWebSocketClient } from './backpackWebSocket';
import { Logger } from '../logger';
import type { AdapterEventListener, Balance, ExchangeAdapter, LeverageValidationResult, Order, OrderRequest, Position } from '../types';

const API_BASE = 'https://api.backpack.exchange';
const DEFAULT_WINDOW_MS = 5000;

type HttpMethod = 'GET' | 'POST' | 'DELETE';

type JsonObject = Record<string, unknown>;

interface BackpackAdapterOptions {
  apiKey?: string;
  apiSecret?: string;
  tradingEnabled?: boolean;
  recvWindowMs?: number;
  enableWebSocket?: boolean;
  wsUrl?: string;
  symbol?: string;
}

const INSTRUCTIONS = {
  accountQuery: 'accountQuery',
  balanceQuery: 'balanceQuery',
  orderQueryAll: 'orderQueryAll',
  positionQuery: 'positionQuery',
  orderExecute: 'orderExecute',
  orderCancel: 'orderCancel',
  markPrices: null
} as const;

function sortEntries(value: JsonObject): [string, unknown][] {
  return Object.entries(value)
    .filter(([, entry]) => entry !== undefined && entry !== null)
    .sort(([a], [b]) => a.localeCompare(b));
}

function encodeValue(value: unknown): string {
  if (typeof value === 'boolean') return value ? 'true' : 'false';
  return String(value);
}

function buildQuery(value: JsonObject): string {
  return sortEntries(value)
    .map(([key, entry]) => `${encodeURIComponent(key)}=${encodeURIComponent(encodeValue(entry))}`)
    .join('&');
}

function fromBase64Seed(seed: string): crypto.KeyObject {
  const rawPrivate = Buffer.from(seed, 'base64').subarray(0, 32);
  const prefixPrivateEd25519 = Buffer.from('302e020100300506032b657004220420', 'hex');
  const der = Buffer.concat([prefixPrivateEd25519, rawPrivate]);
  return crypto.createPrivateKey({ key: der, format: 'der', type: 'pkcs8' });
}

function mapSide(side: string): 'buy' | 'sell' {
  return side === 'Ask' ? 'sell' : 'buy';
}

function mapOrderStatus(status: string | undefined): Order['status'] {
  switch (status) {
    case 'Filled':
    case 'orderFill':
      return 'filled';
    case 'Cancelled':
    case 'Canceled':
    case 'orderCancelled':
      return 'cancelled';
    case 'Rejected':
      return 'rejected';
    case 'Accepted':
    case 'New':
    case 'Open':
    default:
      return 'open';
  }
}

function mapOrderType(orderType: string | undefined): Order['type'] {
  return orderType?.toLowerCase() === 'market' ? 'market' : 'limit';
}

export class BackpackFuturesAdapter implements ExchangeAdapter {
  readonly name = 'backpack-futures';
  private readonly logger = new Logger('BackpackFuturesAdapter');
  private readonly apiKey?: string;
  private readonly apiSecret?: crypto.KeyObject;
  private readonly tradingEnabled: boolean;
  private readonly recvWindowMs: number;
  private readonly webSocketEnabled: boolean;
  private readonly webSocket?: BackpackWebSocketClient;
  private connected = false;
  private readonly eventListeners = new Set<AdapterEventListener>();

  constructor(options: BackpackAdapterOptions = {}) {
    this.apiKey = options.apiKey ?? process.env.BACKPACK_API_KEY;
    this.tradingEnabled = options.tradingEnabled ?? ['1', 'true', 'yes', 'on'].includes((process.env.BACKPACK_ENABLE_LIVE ?? '').toLowerCase());
    this.recvWindowMs = Math.max(1, Math.min(60_000, options.recvWindowMs ?? Number(process.env.BACKPACK_WINDOW_MS ?? DEFAULT_WINDOW_MS)));
    this.webSocketEnabled = options.enableWebSocket ?? ['1', 'true', 'yes', 'on'].includes((process.env.BACKPACK_ENABLE_WS ?? 'true').toLowerCase());

    const seed = options.apiSecret ?? process.env.BACKPACK_API_SECRET;
    this.apiSecret = seed ? fromBase64Seed(seed) : undefined;

    if (this.webSocketEnabled) {
      this.webSocket = new BackpackWebSocketClient({
        apiKey: this.apiKey,
        apiSecret: this.apiSecret,
        liveEnabled: this.tradingEnabled,
        wsUrl: options.wsUrl,
        symbol: options.symbol ?? process.env.GRID_SYMBOL
      });
      this.webSocket.onEvent((event) => {
        for (const listener of this.eventListeners) listener(event);
      });
    }
  }

  onEvent(listener: AdapterEventListener): () => void {
    this.eventListeners.add(listener);
    return () => this.eventListeners.delete(listener);
  }

  getTradingAccessMode(): 'read-only' | 'live' {
    return this.tradingEnabled ? 'live' : 'read-only';
  }

  async connect(): Promise<void> {
    await this.publicRequest<JsonObject>('GET', '/api/v1/time');
    if (this.webSocket) {
      try {
        await this.webSocket.connect();
      } catch (error) {
        this.logger.warn('Backpack WebSocket connect failed; continuing with REST only.', {
          error: error instanceof Error ? error.message : String(error)
        });
      }
    }
    this.connected = true;
    this.logger.info('Connected to Backpack REST API.', { recvWindowMs: this.recvWindowMs, websocket: Boolean(this.webSocket), tradingAccessMode: this.getTradingAccessMode() });
  }

  async disconnect(): Promise<void> {
    await this.webSocket?.disconnect();
    this.connected = false;
    this.logger.info('Backpack adapter disconnected.');
  }

  async getBalance(asset: string): Promise<Balance> {
    this.assertConnected();
    this.assertCredentials('signed read balance');
    const payload = await this.signedRequest<unknown>('GET', '/api/v1/capital', INSTRUCTIONS.balanceQuery, {});
    const balances = this.normalizeBalances(payload);
    const row = balances.find((entry) => String(entry.asset ?? '') === asset);
    if (!row) {
      return { asset, total: 0, available: 0 };
    }
    return {
      asset,
      total: Number(row.total ?? row.balance ?? 0),
      available: Number(row.available ?? row.availableBalance ?? row.free ?? 0)
    };
  }

  async getOpenOrders(symbol: string): Promise<Order[]> {
    this.assertConnected();
    this.assertCredentials('signed read open orders');
    const payload = await this.signedRequest<unknown[]>('GET', '/api/v1/orders', INSTRUCTIONS.orderQueryAll, { symbol });
    return payload
      .filter((entry) => typeof entry === 'object' && entry !== null)
      .map((entry) => this.normalizeOrder(entry as JsonObject))
      .filter((order) => order.status === 'open');
  }

  async getPosition(symbol: string): Promise<Position> {
    this.assertConnected();
    this.assertCredentials('signed read position');
    try {
      const payload = await this.signedRequest<JsonObject>('GET', '/api/v1/position', INSTRUCTIONS.positionQuery, { symbol });
      const netQuantity = Number(payload.netQuantity ?? payload.quantity ?? payload.positionQty ?? 0);
      const entryPrice = Number(payload.entryPrice ?? payload.averageEntryPrice ?? 0);
      const unrealizedPnl = Number(payload.unrealizedPnl ?? payload.pnl ?? 0);
      return { symbol, size: netQuantity, entryPrice, unrealizedPnl };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (message.includes('404') && message.includes('RESOURCE_NOT_FOUND')) {
        return { symbol, size: 0, entryPrice: 0, unrealizedPnl: 0 };
      }
      throw error;
    }
  }

  async placeOrder(request: OrderRequest): Promise<Order> {
    this.assertConnected();
    this.assertTradingEnabled('placeOrder');
    const side = request.side === 'buy' ? 'Bid' : 'Ask';
    const payload: JsonObject = {
      symbol: request.symbol,
      side,
      orderType: request.type === 'market' ? 'Market' : 'Limit',
      quantity: String(request.qty),
      clientId: request.clientOrderId,
      reduceOnly: request.reduceOnly ?? false,
      postOnly: request.postOnly ?? false
    };
    if (request.type === 'limit') {
      payload.price = String(request.price);
      payload.timeInForce = 'GTC';
    }

    const response = await this.signedRequest<JsonObject>('POST', '/api/v1/order', INSTRUCTIONS.orderExecute, payload);
    return this.normalizeOrder(response, request);
  }

  async cancelOrder(symbol: string, orderId: string): Promise<void> {
    this.assertConnected();
    this.assertTradingEnabled('cancelOrder');
    await this.signedRequest('DELETE', '/api/v1/order', INSTRUCTIONS.orderCancel, { symbol, orderId });
  }

  async markPrice(symbol: string): Promise<number> {
    this.assertConnected();
    const payload = await this.publicRequest<unknown[]>('GET', '/api/v1/markPrices');
    const row = payload.find((entry) => typeof entry === 'object' && entry !== null && String((entry as JsonObject).symbol ?? '') === symbol) as JsonObject | undefined;
    if (!row) {
      throw new Error(`No mark price found for symbol ${symbol}`);
    }
    return Number(row.markPrice ?? row.price);
  }

  async validateLeverage(symbol: string, target: number): Promise<LeverageValidationResult> {
    this.assertConnected();
    this.assertCredentials('account leverage validation');
    const account = await this.signedRequest<JsonObject>('GET', '/api/v1/account', INSTRUCTIONS.accountQuery, {});
    const accountLimit = Number(account.leverageLimit ?? 0);
    if (Number.isFinite(accountLimit) && accountLimit > 0 && target > accountLimit) {
      return {
        target,
        accountLimit,
        verified: false,
        canSet: false,
        accepted: false,
        message: `Configured GRID_LEVERAGE=${target} exceeds Backpack account leverageLimit=${accountLimit} for ${symbol}.`
      };
    }
    return {
      target,
      accountLimit: Number.isFinite(accountLimit) && accountLimit > 0 ? accountLimit : undefined,
      verified: false,
      canSet: false,
      accepted: true,
      message: `Backpack API exposes leverageLimit${Number.isFinite(accountLimit) && accountLimit > 0 ? `=${accountLimit}` : ''} but not the currently applied per-market leverage for ${symbol}; set leverage manually on the exchange side to ${target}x before enabling live mode.`
    };
  }

  private assertConnected(): void {
    if (!this.connected) {
      throw new Error('Backpack adapter is not connected. Call connect() first.');
    }
  }

  private assertCredentials(operation: string): void {
    if (!this.apiKey || !this.apiSecret) {
      throw new Error(`Backpack credentials missing for ${operation}. Expected BACKPACK_API_KEY and BACKPACK_API_SECRET.`);
    }
  }

  private assertTradingEnabled(operation: string): void {
    this.assertCredentials(operation);
    if (!this.tradingEnabled) {
      throw new Error(`Backpack adapter is in read-only mode; refusing to ${operation}. Set BACKPACK_ENABLE_LIVE=true to enable order mutations.`);
    }
  }

  private normalizeOrder(payload: JsonObject, fallback?: Partial<OrderRequest>): Order {
    return {
      orderId: String(payload.orderId ?? payload.id ?? payload.clientId ?? fallback?.clientOrderId ?? 'unknown'),
      clientOrderId: String(payload.clientId ?? payload.clientOrderId ?? fallback?.clientOrderId ?? ''),
      symbol: String(payload.symbol ?? fallback?.symbol ?? ''),
      side: mapSide(String(payload.side ?? (fallback?.side === 'sell' ? 'Ask' : 'Bid'))),
      type: mapOrderType(String(payload.orderType ?? fallback?.type ?? 'limit')),
      status: mapOrderStatus(typeof payload.status === 'string' ? payload.status : undefined),
      price: payload.price !== undefined ? Number(payload.price) : fallback?.price,
      qty: Number(payload.quantity ?? payload.qty ?? fallback?.qty ?? 0),
      filledQty: Number(payload.executedQuantity ?? payload.filledQuantity ?? payload.filledQty ?? 0),
      reduceOnly: Boolean(payload.reduceOnly ?? fallback?.reduceOnly ?? false),
      postOnly: Boolean(payload.postOnly ?? fallback?.postOnly ?? false),
      ts: Number(payload.timestamp ?? payload.createdAt ?? Date.now())
    };
  }

  private signature(instruction: string, params: JsonObject, timestamp: number): string {
    if (!this.apiSecret) {
      throw new Error('Backpack private key unavailable.');
    }
    const messageCore = buildQuery(params);
    const message = `instruction=${instruction}${messageCore ? `&${messageCore}` : ''}&timestamp=${timestamp}&window=${this.recvWindowMs}`;
    return crypto.sign(null, Buffer.from(message), this.apiSecret).toString('base64');
  }

  private async publicRequest<T>(method: HttpMethod, path: string, params?: JsonObject): Promise<T> {
    return this.request<T>(method, path, params);
  }

  private async signedRequest<T>(method: HttpMethod, path: string, instruction: string, params: JsonObject): Promise<T> {
    this.assertCredentials(`signed request ${instruction}`);
    const apiKey = this.apiKey;
    if (!apiKey) {
      throw new Error('Backpack API key unavailable.');
    }
    const timestamp = Date.now();
    const headers = {
      'X-Timestamp': String(timestamp),
      'X-Window': String(this.recvWindowMs),
      'X-API-Key': apiKey,
      'X-Signature': this.signature(instruction, params, timestamp)
    };
    return this.request<T>(method, path, params, headers);
  }

  private async request<T>(method: HttpMethod, path: string, params: JsonObject = {}, headers: Record<string, string> = {}): Promise<T> {
    const url = new URL(`${API_BASE}${path}`);
    const init: RequestInit = {
      method,
      headers: {
        'User-Agent': 'backpack-grid-bot/0.1',
        'Content-Type': 'application/json; charset=utf-8',
        ...headers
      }
    };

    if (method === 'GET') {
      for (const [key, value] of sortEntries(params)) {
        url.searchParams.set(key, encodeValue(value));
      }
    } else {
      init.body = JSON.stringify(params);
    }

    const response = await fetch(url, init);
    const text = await response.text();
    const body = text.length ? this.parseJson(text) : null;

    if (!response.ok) {
      throw new Error(`Backpack API ${method} ${path} failed: ${response.status} ${response.statusText} ${text}`);
    }
    return body as T;
  }

  private normalizeBalances(payload: unknown): JsonObject[] {
    if (Array.isArray(payload)) {
      return payload.filter((entry): entry is JsonObject => typeof entry === 'object' && entry !== null);
    }
    if (typeof payload !== 'object' || payload === null) {
      return [];
    }

    const record = payload as JsonObject;
    const nested = record.balances ?? record.capital ?? record.items;
    if (Array.isArray(nested)) {
      return nested.filter((entry): entry is JsonObject => typeof entry === 'object' && entry !== null);
    }

    return Object.entries(record)
      .filter(([, entry]) => typeof entry === 'object' && entry !== null)
      .map(([asset, entry]) => ({ asset, ...(entry as JsonObject) }));
  }

  private parseJson(text: string): unknown {
    try {
      return JSON.parse(text);
    } catch {
      return text;
    }
  }
}
