import crypto from 'node:crypto';
import { Logger } from '../logger';
import type { AdapterEvent, AdapterEventListener, Fill, Order, Position } from '../types';

const DEFAULT_WS_URL = 'wss://ws.backpack.exchange';

type JsonObject = Record<string, unknown>;
type WsChannel = 'markPrices' | 'orders' | 'positions';

export interface BackpackWebSocketOptions {
  wsUrl?: string;
  apiKey?: string;
  apiSecret?: crypto.KeyObject;
  liveEnabled?: boolean;
  symbol?: string;
}

interface SubscribeRequest {
  channel: WsChannel;
  symbol?: string;
  authenticated?: boolean;
}

function normalizeTs(value: unknown): number {
  const n = Number(value ?? Date.now());
  return Number.isFinite(n) ? n : Date.now();
}

function sideOf(value: unknown): 'buy' | 'sell' {
  return String(value ?? 'Bid') === 'Ask' ? 'sell' : 'buy';
}

function orderTypeOf(value: unknown): 'limit' | 'market' {
  return String(value ?? 'Limit').toLowerCase() === 'market' ? 'market' : 'limit';
}

function statusOf(value: unknown): Order['status'] {
  switch (String(value ?? 'Open')) {
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

export class BackpackWebSocketClient {
  private readonly logger = new Logger('BackpackWebSocketClient');
  private readonly wsUrl: string;
  private readonly apiKey?: string;
  private readonly apiSecret?: crypto.KeyObject;
  private readonly liveEnabled: boolean;
  private readonly symbol?: string;
  private readonly listeners = new Set<AdapterEventListener>();
  private socket?: WebSocket;

  constructor(options: BackpackWebSocketOptions = {}) {
    this.wsUrl = options.wsUrl ?? process.env.BACKPACK_WS_URL ?? DEFAULT_WS_URL;
    this.apiKey = options.apiKey;
    this.apiSecret = options.apiSecret;
    this.liveEnabled = options.liveEnabled ?? false;
    this.symbol = options.symbol;
  }

  onEvent(listener: AdapterEventListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  async connect(): Promise<void> {
    if (!this.liveEnabled) {
      this.logger.warn('WebSocket client left disabled by safety flag.');
      return;
    }

    this.emit({ kind: 'connection', state: 'connecting' });
    const socket = new WebSocket(this.wsUrl);
    this.socket = socket;

    await new Promise<void>((resolve, reject) => {
      const onOpen = () => {
        cleanup();
        resolve();
      };
      const onError = (event: Event) => {
        cleanup();
        reject(new Error(`Backpack WebSocket connection failed: ${String((event as unknown as { message?: string }).message ?? 'unknown error')}`));
      };
      const cleanup = () => {
        socket.removeEventListener('open', onOpen);
        socket.removeEventListener('error', onError);
      };
      socket.addEventListener('open', onOpen, { once: true });
      socket.addEventListener('error', onError, { once: true });
    });

    socket.addEventListener('message', (event) => this.handleMessage(typeof event.data === 'string' ? event.data : ''));
    socket.addEventListener('close', () => this.emit({ kind: 'connection', state: 'disconnected' }));
    socket.addEventListener('error', () => this.emit({ kind: 'connection', state: 'error', message: 'websocket error' }));

    this.emit({ kind: 'connection', state: 'connected' });
    this.subscribe({ channel: 'markPrices', symbol: this.symbol });
    if (this.apiKey && this.apiSecret) {
      this.subscribe({ channel: 'orders', symbol: this.symbol, authenticated: true });
      this.subscribe({ channel: 'positions', symbol: this.symbol, authenticated: true });
    }
  }

  async disconnect(): Promise<void> {
    this.socket?.close();
    this.socket = undefined;
  }

  private subscribe(request: SubscribeRequest): void {
    const payload: JsonObject = {
      method: 'SUBSCRIBE',
      params: {
        channel: request.channel,
        ...(request.symbol ? { symbol: request.symbol } : {}),
        ...(request.authenticated ? this.buildAuthPayload(request.channel) : {})
      }
    };
    this.send(payload);
  }

  private buildAuthPayload(channel: string): JsonObject {
    const timestamp = Date.now();
    const nonce = crypto.randomUUID();
    const instruction = `subscribe:${channel}`;
    const signature = this.apiSecret
      ? crypto.sign(null, Buffer.from(`${instruction}:${timestamp}:${nonce}`), this.apiSecret).toString('base64')
      : undefined;
    return {
      apiKey: this.apiKey,
      timestamp,
      nonce,
      signature
    };
  }

  private send(payload: JsonObject): void {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) return;
    this.socket.send(JSON.stringify(payload));
  }

  private handleMessage(text: string): void {
    if (!text) return;
    let payload: unknown;
    try {
      payload = JSON.parse(text);
    } catch {
      this.logger.warn('Ignoring non-JSON WebSocket payload.');
      return;
    }
    const event = this.normalizeEvent(payload);
    if (event) this.emit(event);
  }

  private normalizeEvent(payload: unknown): AdapterEvent | undefined {
    if (!payload || typeof payload !== 'object') return undefined;
    const row = payload as JsonObject;
    const channel = String(row.channel ?? row.stream ?? row.topic ?? '');
    const data = (row.data ?? row.payload ?? row.result ?? row) as JsonObject;

    if (channel.toLowerCase().includes('order')) {
      return { kind: 'order', order: this.normalizeOrder(data), raw: payload };
    }
    if (channel.toLowerCase().includes('position')) {
      return { kind: 'position', position: this.normalizePosition(data), raw: payload };
    }
    if (channel.toLowerCase().includes('fill') || String(data.event ?? '').toLowerCase().includes('fill')) {
      return { kind: 'fill', fill: this.normalizeFill(data), raw: payload };
    }
    return undefined;
  }

  private normalizeOrder(payload: JsonObject): Order {
    return {
      orderId: String(payload.orderId ?? payload.id ?? payload.clientId ?? 'unknown'),
      clientOrderId: String(payload.clientId ?? payload.clientOrderId ?? ''),
      symbol: String(payload.symbol ?? this.symbol ?? ''),
      side: sideOf(payload.side),
      type: orderTypeOf(payload.orderType),
      status: statusOf(payload.status),
      price: payload.price === undefined ? undefined : Number(payload.price),
      qty: Number(payload.quantity ?? payload.qty ?? 0),
      filledQty: Number(payload.executedQuantity ?? payload.filledQuantity ?? payload.filledQty ?? 0),
      reduceOnly: Boolean(payload.reduceOnly ?? false),
      postOnly: Boolean(payload.postOnly ?? false),
      ts: normalizeTs(payload.timestamp ?? payload.createdAt ?? payload.updatedAt)
    };
  }

  private normalizePosition(payload: JsonObject): Position {
    return {
      symbol: String(payload.symbol ?? this.symbol ?? ''),
      size: Number(payload.netQuantity ?? payload.quantity ?? payload.positionQty ?? 0),
      entryPrice: Number(payload.entryPrice ?? payload.averageEntryPrice ?? 0),
      unrealizedPnl: Number(payload.unrealizedPnl ?? payload.pnl ?? 0)
    };
  }

  private normalizeFill(payload: JsonObject): Fill {
    return {
      orderId: String(payload.orderId ?? payload.id ?? 'unknown'),
      clientOrderId: String(payload.clientId ?? payload.clientOrderId ?? ''),
      symbol: String(payload.symbol ?? this.symbol ?? ''),
      side: sideOf(payload.side),
      price: Number(payload.price ?? 0),
      qty: Number(payload.quantity ?? payload.qty ?? 0),
      fee: Number(payload.fee ?? 0),
      ts: normalizeTs(payload.timestamp ?? payload.createdAt ?? payload.updatedAt)
    };
  }

  private emit(event: AdapterEvent): void {
    for (const listener of this.listeners) listener(event);
  }
}
