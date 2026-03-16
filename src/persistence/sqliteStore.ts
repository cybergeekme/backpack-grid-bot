import fs from 'node:fs';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';

import { Logger } from '../logger';
import type {
  Fill,
  Order,
  Position,
  RuntimeCheckpoint,
  RuntimeState,
  StrategySnapshot,
  ReconciliationEvent,
  RiskEvent
} from '../types';

interface RecoveryBundle {
  state: RuntimeState;
  checkpoint?: RuntimeCheckpoint;
}

const CHECKPOINT_RETENTION: Record<string, number> = {
  runtime: 200,
  position: 500,
  connection: 200
};

function parseJson<T>(value: string): T {
  return JSON.parse(value) as T;
}

export class SqlitePersistence {
  private readonly logger = new Logger('SqlitePersistence');
  private readonly db: DatabaseSync;

  constructor(private readonly dbPath: string, private readonly serviceName: string) {
    fs.mkdirSync(path.dirname(dbPath), { recursive: true });
    this.db = new DatabaseSync(dbPath);
    this.db.exec('PRAGMA journal_mode = WAL;');
    this.db.exec('PRAGMA synchronous = NORMAL;');
    this.db.exec('PRAGMA foreign_keys = ON;');
    this.initSchema();
  }

  close(): void {
    this.db.close();
  }

  loadLatestRecovery(): RecoveryBundle {
    const snapshotRow = this.db
      .prepare(
        `SELECT payload FROM strategy_snapshots
         WHERE service_name = ?
         ORDER BY created_ts DESC, id DESC
         LIMIT 1`
      )
      .get(this.serviceName) as { payload: string } | undefined;

    const positionRow = this.db
      .prepare(
        `SELECT payload FROM service_checkpoints
         WHERE service_name = ? AND checkpoint_kind = 'position'
         ORDER BY created_ts DESC, id DESC
         LIMIT 1`
      )
      .get(this.serviceName) as { payload: string } | undefined;

    const fillRows = this.db
      .prepare(
        `SELECT payload FROM fills
         WHERE service_name = ?
         ORDER BY ts DESC, id DESC
         LIMIT 100`
      )
      .all(this.serviceName) as { payload: string }[];

    const orderRows = this.db
      .prepare(
        `SELECT payload FROM orders
         WHERE service_name = ? AND is_working = 1
         ORDER BY updated_ts DESC, id DESC`
      )
      .all(this.serviceName) as { payload: string }[];

    const checkpointRow = this.db
      .prepare(
        `SELECT payload FROM service_checkpoints
         WHERE service_name = ? AND checkpoint_kind = 'runtime'
         ORDER BY created_ts DESC, id DESC
         LIMIT 1`
      )
      .get(this.serviceName) as { payload: string } | undefined;

    const checkpoint = checkpointRow ? parseJson<RuntimeCheckpoint>(checkpointRow.payload) : undefined;

    return {
      state: {
        snapshot: checkpoint?.state.snapshot ?? (snapshotRow ? parseJson<StrategySnapshot>(snapshotRow.payload) : undefined),
        position: checkpoint?.state.position ?? (positionRow ? parseJson<Position>(positionRow.payload) : undefined),
        workingOrders: orderRows.map((row) => parseJson<Order>(row.payload)),
        recentFills: fillRows.map((row) => parseJson<Fill>(row.payload))
      },
      checkpoint
    };
  }

  persistSnapshot(snapshot: StrategySnapshot): void {
    this.db
      .prepare('INSERT INTO strategy_snapshots(service_name, symbol, mid_price, payload, created_ts) VALUES (?, ?, ?, ?, ?)')
      .run(this.serviceName, snapshot.symbol, snapshot.midPrice, JSON.stringify(snapshot), Date.now());
  }

  persistOrders(orders: Order[]): void {
    const clear = this.db.prepare('UPDATE orders SET is_working = 0, updated_ts = ? WHERE service_name = ?');
    const upsert = this.db.prepare(
      `INSERT INTO orders(service_name, order_id, client_order_id, symbol, side, status, price, qty, filled_qty, is_working, payload, updated_ts)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(service_name, order_id) DO UPDATE SET
         client_order_id = excluded.client_order_id,
         symbol = excluded.symbol,
         side = excluded.side,
         status = excluded.status,
         price = excluded.price,
         qty = excluded.qty,
         filled_qty = excluded.filled_qty,
         is_working = excluded.is_working,
         payload = excluded.payload,
         updated_ts = excluded.updated_ts`
    );

    const now = Date.now();
    this.db.exec('BEGIN');
    try {
      clear.run(now, this.serviceName);
      for (const order of orders) {
        upsert.run(
          this.serviceName,
          order.orderId,
          order.clientOrderId,
          order.symbol,
          order.side,
          order.status,
          order.price ?? null,
          order.qty,
          order.filledQty,
          order.status === 'open' || order.status === 'new' ? 1 : 0,
          JSON.stringify(order),
          now
        );
      }
      this.db.exec('COMMIT');
    } catch (error) {
      this.db.exec('ROLLBACK');
      throw error;
    }
  }

  persistFill(fill: Fill): void {
    this.db
      .prepare(
        'INSERT INTO fills(service_name, order_id, client_order_id, symbol, side, price, qty, fee, ts, payload) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)'
      )
      .run(this.serviceName, fill.orderId, fill.clientOrderId, fill.symbol, fill.side, fill.price, fill.qty, fill.fee, fill.ts, JSON.stringify(fill));
  }

  persistReconciliation(event: ReconciliationEvent): void {
    this.db
      .prepare(
        'INSERT INTO reconciliation_events(service_name, symbol, matched, missing_count, unexpected_count, payload, created_ts) VALUES (?, ?, ?, ?, ?, ?, ?)'
      )
      .run(
        this.serviceName,
        event.symbol,
        event.matched,
        event.missingOnExchange.length,
        event.unexpectedOnExchange.length,
        JSON.stringify(event),
        event.ts
      );
  }

  persistRiskEvent(event: RiskEvent): void {
    this.db
      .prepare(
        'INSERT INTO risk_events(service_name, symbol, side, qty, reason, payload, created_ts) VALUES (?, ?, ?, ?, ?, ?, ?)'
      )
      .run(this.serviceName, event.symbol, event.side, event.qty, event.reason, JSON.stringify(event), event.ts);
  }

  persistPosition(position: Position): void {
    this.persistCheckpoint('position', position);
  }

  persistCheckpoint(kind: string, payload: unknown): void {
    const now = Date.now();
    this.db
      .prepare('INSERT INTO service_checkpoints(service_name, checkpoint_kind, payload, created_ts) VALUES (?, ?, ?, ?)')
      .run(this.serviceName, kind, JSON.stringify(payload), now);
    this.pruneCheckpoints(kind);
  }

  private pruneCheckpoints(kind: string): void {
    const keep = CHECKPOINT_RETENTION[kind];
    if (!keep) return;
    this.db
      .prepare(
        `DELETE FROM service_checkpoints
         WHERE service_name = ?
           AND checkpoint_kind = ?
           AND id NOT IN (
             SELECT id FROM service_checkpoints
             WHERE service_name = ? AND checkpoint_kind = ?
             ORDER BY created_ts DESC, id DESC
             LIMIT ?
           )`
      )
      .run(this.serviceName, kind, this.serviceName, kind, keep);
  }

  private initSchema(): void {
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS strategy_snapshots (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        service_name TEXT NOT NULL,
        symbol TEXT NOT NULL,
        mid_price REAL NOT NULL,
        payload TEXT NOT NULL,
        created_ts INTEGER NOT NULL
      );
      CREATE INDEX IF NOT EXISTS idx_strategy_snapshots_service_created ON strategy_snapshots(service_name, created_ts DESC);

      CREATE TABLE IF NOT EXISTS orders (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        service_name TEXT NOT NULL,
        order_id TEXT NOT NULL,
        client_order_id TEXT NOT NULL,
        symbol TEXT NOT NULL,
        side TEXT NOT NULL,
        status TEXT NOT NULL,
        price REAL,
        qty REAL NOT NULL,
        filled_qty REAL NOT NULL,
        is_working INTEGER NOT NULL DEFAULT 0,
        payload TEXT NOT NULL,
        updated_ts INTEGER NOT NULL,
        UNIQUE(service_name, order_id)
      );
      CREATE INDEX IF NOT EXISTS idx_orders_service_working ON orders(service_name, is_working, updated_ts DESC);

      CREATE TABLE IF NOT EXISTS fills (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        service_name TEXT NOT NULL,
        order_id TEXT NOT NULL,
        client_order_id TEXT NOT NULL,
        symbol TEXT NOT NULL,
        side TEXT NOT NULL,
        price REAL NOT NULL,
        qty REAL NOT NULL,
        fee REAL NOT NULL,
        ts INTEGER NOT NULL,
        payload TEXT NOT NULL
      );
      CREATE INDEX IF NOT EXISTS idx_fills_service_ts ON fills(service_name, ts DESC);

      CREATE TABLE IF NOT EXISTS reconciliation_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        service_name TEXT NOT NULL,
        symbol TEXT NOT NULL,
        matched INTEGER NOT NULL,
        missing_count INTEGER NOT NULL,
        unexpected_count INTEGER NOT NULL,
        payload TEXT NOT NULL,
        created_ts INTEGER NOT NULL
      );
      CREATE INDEX IF NOT EXISTS idx_reconciliation_service_ts ON reconciliation_events(service_name, created_ts DESC);

      CREATE TABLE IF NOT EXISTS risk_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        service_name TEXT NOT NULL,
        symbol TEXT NOT NULL,
        side TEXT NOT NULL,
        qty REAL NOT NULL,
        reason TEXT NOT NULL,
        payload TEXT NOT NULL,
        created_ts INTEGER NOT NULL
      );
      CREATE INDEX IF NOT EXISTS idx_risk_service_ts ON risk_events(service_name, created_ts DESC);

      CREATE TABLE IF NOT EXISTS service_checkpoints (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        service_name TEXT NOT NULL,
        checkpoint_kind TEXT NOT NULL,
        payload TEXT NOT NULL,
        created_ts INTEGER NOT NULL
      );
      CREATE INDEX IF NOT EXISTS idx_checkpoints_service_kind_ts ON service_checkpoints(service_name, checkpoint_kind, created_ts DESC);
    `);
    this.logger.info('SQLite schema ready.', { dbPath: this.dbPath, wal: true, serviceName: this.serviceName });
  }
}
