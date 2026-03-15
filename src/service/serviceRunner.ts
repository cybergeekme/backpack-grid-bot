import { setTimeout as delay } from 'node:timers/promises';

import type { AppConfig } from '../config';
import { Logger } from '../logger';
import { GridTradingService } from './gridService';

export type ServiceHealthState = 'STARTING' | 'ACTIVE' | 'DEGRADED' | 'SAFE_MODE' | 'PAUSED' | 'STOPPED';

export interface ServiceStatusSnapshot {
  state: ServiceHealthState;
  startedAt?: number;
  lastLoopAt?: number;
  lastSuccessAt?: number;
  lastReconcileAt?: number;
  lastErrorAt?: number;
  consecutiveErrors: number;
  pauseReason?: string;
  safeModeReason?: string;
}

export class ServiceRunner {
  private readonly logger = new Logger('ServiceRunner');
  private state: ServiceHealthState = 'STOPPED';
  private status: ServiceStatusSnapshot = {
    state: 'STOPPED',
    consecutiveErrors: 0
  };
  private stopping = false;
  private activeLoop?: Promise<void>;
  private nextReconcileTs = 0;

  constructor(
    private readonly config: AppConfig,
    private readonly service: GridTradingService,
    private readonly hooks: { onMockTick?: () => void } = {}
  ) {}

  async start(): Promise<void> {
    if (this.activeLoop) return this.activeLoop;
    this.stopping = false;
    this.transition('STARTING');
    this.status.startedAt = Date.now();
    this.activeLoop = this.run();
    return this.activeLoop;
  }

  async stop(reason = 'operator_stop'): Promise<void> {
    this.stopping = true;
    this.status.pauseReason = reason;
    await this.activeLoop;
  }

  snapshot(): ServiceStatusSnapshot {
    return { ...this.status };
  }

  private async run(): Promise<void> {
    try {
      await this.service.start();
      this.transition('ACTIVE');
      this.nextReconcileTs = Date.now() + this.config.serviceReconcileIntervalMs;

      while (!this.stopping) {
        this.status.lastLoopAt = Date.now();
        try {
          if (this.state === 'PAUSED') {
            await this.runPauseLoop();
          } else if (this.state === 'SAFE_MODE') {
            await this.runSafeModeLoop();
          } else {
            this.hooks.onMockTick?.();
            await this.service.rebalance();
            const runtime = this.service.getRuntimeHealth();
            if (runtime.pauseRequested) {
              this.status.pauseReason = runtime.pauseReason ?? 'paused';
              this.transition('PAUSED', { reason: this.status.pauseReason });
            }
            await this.runPeriodicReconciliation(false);
            this.status.lastSuccessAt = Date.now();
            this.status.consecutiveErrors = 0;
            if (!runtime.pauseRequested && this.state !== 'ACTIVE') {
              this.transition('ACTIVE');
            }
          }
        } catch (error) {
          this.status.lastErrorAt = Date.now();
          this.status.consecutiveErrors += 1;
          const message = error instanceof Error ? error.message : String(error);
          const threshold = this.config.serviceConsecutiveErrorThreshold;
          if (this.status.consecutiveErrors >= threshold) {
            this.status.safeModeReason = `consecutive_error_threshold:${threshold}`;
            this.transition('SAFE_MODE', { error: message, consecutiveErrors: this.status.consecutiveErrors });
          } else {
            this.transition('DEGRADED', { error: message, consecutiveErrors: this.status.consecutiveErrors });
          }
        }

        if (!this.stopping) {
          await this.sleepUntilNextLoop();
        }
      }
    } finally {
      await this.service.stop();
      this.transition('STOPPED');
      this.activeLoop = undefined;
    }
  }

  private async runPauseLoop(): Promise<void> {
    await this.runPeriodicReconciliation(true);
    const runtime = this.service.getRuntimeHealth();
    if (!runtime.pauseRequested) {
      this.status.pauseReason = undefined;
      this.transition('ACTIVE', { reason: 'pause_cleared' });
      return;
    }
    this.status.pauseReason = runtime.pauseReason ?? 'paused';
    this.logger.warn('Service remains paused.', { reason: this.status.pauseReason });
  }

  private async runSafeModeLoop(): Promise<void> {
    await this.runPeriodicReconciliation(true);
    this.logger.warn('Service in SAFE_MODE; rebalance disabled pending operator review.', {
      safeModeReason: this.status.safeModeReason,
      consecutiveErrors: this.status.consecutiveErrors
    });
  }

  private async runPeriodicReconciliation(force: boolean): Promise<void> {
    const now = Date.now();
    if (!force && now < this.nextReconcileTs) return;
    await this.service.reconcileWithExchange();
    this.status.lastReconcileAt = Date.now();
    this.nextReconcileTs = Date.now() + this.config.serviceReconcileIntervalMs;
    const runtime = this.service.getRuntimeHealth();
    if (runtime.pauseRequested) {
      this.status.pauseReason = runtime.pauseReason ?? 'paused';
      this.transition('PAUSED', { reason: this.status.pauseReason });
    }
  }

  private async sleepUntilNextLoop(): Promise<void> {
    const chunkMs = Math.min(250, this.config.serviceLoopIntervalMs);
    let remaining = this.config.serviceLoopIntervalMs;
    while (!this.stopping && remaining > 0) {
      await delay(Math.min(chunkMs, remaining));
      remaining -= chunkMs;
    }
  }

  private transition(state: ServiceHealthState, data?: Record<string, unknown>): void {
    if (this.state === state) {
      if (state !== 'ACTIVE' && data) this.logger.warn('Service state unchanged.', { state, ...data });
      this.status.state = state;
      return;
    }
    this.state = state;
    this.status.state = state;
    const payload = { state, ...data };
    if (state === 'ACTIVE' || state === 'STOPPED') {
      this.logger.info('Service state changed.', payload);
      return;
    }
    this.logger.warn('Service state changed.', payload);
  }
}
