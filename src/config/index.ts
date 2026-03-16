import path from 'node:path';

export interface AppConfig {
  env: 'dev' | 'prod' | 'test';
  symbol: string;
  levels: number;
  spacingBps: number;
  orderSize: number;
  maxPositionAbs: number;
  quoteAsset: string;
  killSwitch: boolean;
  useBackpack: boolean;
  backpackTradingEnabled: boolean;
  backpackWsEnabled: boolean;
  backpackApiKey?: string;
  backpackApiSecret?: string;
  backpackWindowMs: number;
  persistencePath: string;
  serviceName: string;
  serviceLoopIntervalMs: number;
  serviceReconcileIntervalMs: number;
  serviceConsecutiveErrorThreshold: number;
  serviceOutOfRangePauseBps: number;
  serviceMockPriceStep: number;
  telegramAlertsEnabled: boolean;
  telegramBotToken?: string;
  telegramChatId?: string;
  telegramAlertLevel: 'info' | 'warn' | 'critical';
  telegramAlertDedupMs: number;
  telegramNotifyFills: boolean;
}

function num(name: string, fallback: number): number {
  const raw = process.env[name];
  if (!raw) return fallback;
  const parsed = Number(raw);
  if (!Number.isFinite(parsed)) {
    throw new Error(`Invalid numeric env ${name}=${raw}`);
  }
  return parsed;
}

function bool(name: string, fallback = false): boolean {
  const raw = process.env[name];
  if (!raw) return fallback;
  return ['1', 'true', 'yes', 'on'].includes(raw.toLowerCase());
}

function alertLevel(name: string, fallback: 'info' | 'warn' | 'critical'): 'info' | 'warn' | 'critical' {
  const raw = process.env[name];
  if (!raw) return fallback;
  if (raw === 'info' || raw === 'warn' || raw === 'critical') return raw;
  throw new Error(`Invalid alert level env ${name}=${raw}`);
}

export function loadConfig(): AppConfig {
  const envRaw = process.env.NODE_ENV ?? 'dev';
  const env = envRaw === 'production' ? 'prod' : envRaw === 'test' ? 'test' : 'dev';
  return {
    env,
    symbol: process.env.GRID_SYMBOL ?? 'ETH_USDC_PERP',
    levels: Math.max(1, Math.floor(num('GRID_LEVELS', 3))),
    spacingBps: num('GRID_SPACING_BPS', 50),
    orderSize: num('GRID_ORDER_SIZE', 0.01),
    maxPositionAbs: num('GRID_MAX_POSITION_ABS', 0.05),
    quoteAsset: process.env.GRID_QUOTE_ASSET ?? 'USDC',
    killSwitch: bool('GRID_KILL_SWITCH', false),
    useBackpack: bool('GRID_USE_BACKPACK', false),
    backpackTradingEnabled: bool('BACKPACK_ENABLE_LIVE', false),
    backpackWsEnabled: bool('BACKPACK_ENABLE_WS', true),
    backpackApiKey: process.env.BACKPACK_API_KEY,
    backpackApiSecret: process.env.BACKPACK_API_SECRET,
    backpackWindowMs: Math.max(1, Math.min(60_000, Math.floor(num('BACKPACK_WINDOW_MS', 5000)))),
    persistencePath: process.env.GRID_DB_PATH ?? path.resolve(process.cwd(), 'var/backpack-grid-bot.sqlite'),
    serviceName: process.env.GRID_SERVICE_NAME ?? 'backpack-grid-bot',
    serviceLoopIntervalMs: Math.max(250, Math.floor(num('GRID_SERVICE_LOOP_MS', 15_000))),
    serviceReconcileIntervalMs: Math.max(1_000, Math.floor(num('GRID_SERVICE_RECONCILE_MS', 60_000))),
    serviceConsecutiveErrorThreshold: Math.max(1, Math.floor(num('GRID_SERVICE_ERROR_THRESHOLD', 3))),
    serviceOutOfRangePauseBps: Math.max(1, num('GRID_SERVICE_OUT_OF_RANGE_BPS', 75)),
    serviceMockPriceStep: Math.max(0.01, num('GRID_MOCK_PRICE_STEP', 100)),
    telegramAlertsEnabled: bool('TELEGRAM_ALERTS_ENABLED', false),
    telegramBotToken: process.env.TELEGRAM_BOT_TOKEN,
    telegramChatId: process.env.TELEGRAM_CHAT_ID,
    telegramAlertLevel: alertLevel('TELEGRAM_ALERT_LEVEL', 'warn'),
    telegramAlertDedupMs: Math.max(0, Math.floor(num('TELEGRAM_ALERT_DEDUP_MS', 300000))),
    telegramNotifyFills: bool('TELEGRAM_NOTIFY_FILLS', false)
  };
}
