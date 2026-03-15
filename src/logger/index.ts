export type LogLevel = 'debug' | 'info' | 'warn' | 'error';

export class Logger {
  constructor(private readonly scope: string) {}

  private log(level: LogLevel, message: string, data?: Record<string, unknown>): void {
    const line = {
      ts: new Date().toISOString(),
      level,
      scope: this.scope,
      message,
      ...(data ?? {})
    };
    const out = JSON.stringify(line);
    if (level === 'error') {
      console.error(out);
      return;
    }
    console.log(out);
  }

  debug(message: string, data?: Record<string, unknown>): void { this.log('debug', message, data); }
  info(message: string, data?: Record<string, unknown>): void { this.log('info', message, data); }
  warn(message: string, data?: Record<string, unknown>): void { this.log('warn', message, data); }
  error(message: string, data?: Record<string, unknown>): void { this.log('error', message, data); }
}
