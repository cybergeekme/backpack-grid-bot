import { Logger } from '../logger';

export type AlertSeverity = 'info' | 'warn' | 'critical';

export interface AlertEvent {
  key: string;
  severity: AlertSeverity;
  title: string;
  message: string;
  details?: Record<string, unknown>;
  dedupMs?: number;
}

export interface AlertSink {
  notify(event: AlertEvent): Promise<void>;
}

export interface AlertManagerOptions {
  enabled: boolean;
  minSeverity: AlertSeverity;
  dedupMs: number;
}

const SEVERITY_ORDER: Record<AlertSeverity, number> = {
  info: 10,
  warn: 20,
  critical: 30
};

export class AlertManager {
  private readonly logger = new Logger('AlertManager');
  private readonly recent = new Map<string, number>();

  constructor(
    private readonly options: AlertManagerOptions,
    private readonly sinks: AlertSink[] = []
  ) {}

  async notify(event: AlertEvent): Promise<void> {
    if (!this.options.enabled) return;
    if (SEVERITY_ORDER[event.severity] < SEVERITY_ORDER[this.options.minSeverity]) return;

    const dedupWindow = Math.max(0, event.dedupMs ?? this.options.dedupMs);
    const now = Date.now();
    const lastTs = this.recent.get(event.key);
    if (lastTs !== undefined && now - lastTs < dedupWindow) return;
    this.recent.set(event.key, now);
    this.prune(now, dedupWindow);

    for (const sink of this.sinks) {
      try {
        await sink.notify(event);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        this.logger.warn('Alert sink delivery failed.', { key: event.key, severity: event.severity, error: message });
      }
    }
  }

  private prune(now: number, dedupWindow: number): void {
    const cutoff = now - Math.max(dedupWindow, this.options.dedupMs, 60_000);
    for (const [key, ts] of this.recent.entries()) {
      if (ts < cutoff) this.recent.delete(key);
    }
  }
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

export interface TelegramNotifierOptions {
  botToken: string;
  chatId: string;
  serviceName: string;
}

export class TelegramNotifier implements AlertSink {
  constructor(private readonly options: TelegramNotifierOptions) {}

  async notify(event: AlertEvent): Promise<void> {
    const lines = [
      `<b>${escapeHtml(this.options.serviceName)}</b>`,
      `${emojiForSeverity(event.severity)} <b>${escapeHtml(event.title)}</b>`,
      escapeHtml(event.message)
    ];

    if (event.details && Object.keys(event.details).length > 0) {
      lines.push('');
      for (const [key, value] of Object.entries(event.details)) {
        lines.push(`<b>${escapeHtml(key)}</b>: <code>${escapeHtml(formatValue(value))}</code>`);
      }
    }

    const response = await fetch(`https://api.telegram.org/bot${this.options.botToken}/sendMessage`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        chat_id: this.options.chatId,
        text: lines.join('\n'),
        parse_mode: 'HTML',
        disable_web_page_preview: true
      })
    });

    if (!response.ok) {
      const body = await response.text();
      throw new Error(`telegram_send_failed:${response.status}:${body.slice(0, 300)}`);
    }
  }
}

function emojiForSeverity(severity: AlertSeverity): string {
  switch (severity) {
    case 'critical':
      return '🚨';
    case 'warn':
      return '⚠️';
    default:
      return 'ℹ️';
  }
}

function formatValue(value: unknown): string {
  if (value === undefined) return 'undefined';
  if (value === null) return 'null';
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  return JSON.stringify(value);
}
