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
  backpackLiveEnabled: boolean;
  backpackWsEnabled: boolean;
  backpackApiKey?: string;
  backpackApiSecret?: string;
  backpackWindowMs: number;
  persistencePath: string;
  serviceName: string;
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

export function loadConfig(): AppConfig {
  const envRaw = process.env.NODE_ENV ?? 'dev';
  const env = envRaw === 'production' ? 'prod' : envRaw === 'test' ? 'test' : 'dev';
  return {
    env,
    symbol: process.env.GRID_SYMBOL ?? 'BTC_USDC_PERP',
    levels: Math.max(1, Math.floor(num('GRID_LEVELS', 3))),
    spacingBps: num('GRID_SPACING_BPS', 50),
    orderSize: num('GRID_ORDER_SIZE', 0.01),
    maxPositionAbs: num('GRID_MAX_POSITION_ABS', 0.05),
    quoteAsset: process.env.GRID_QUOTE_ASSET ?? 'USDC',
    killSwitch: bool('GRID_KILL_SWITCH', false),
    useBackpack: bool('GRID_USE_BACKPACK', false),
    backpackLiveEnabled: bool('BACKPACK_ENABLE_LIVE', false),
    backpackWsEnabled: bool('BACKPACK_ENABLE_WS', true),
    backpackApiKey: process.env.BACKPACK_API_KEY,
    backpackApiSecret: process.env.BACKPACK_API_SECRET,
    backpackWindowMs: Math.max(1, Math.min(60_000, Math.floor(num('BACKPACK_WINDOW_MS', 5000)))),
    persistencePath: process.env.GRID_DB_PATH ?? path.resolve(process.cwd(), 'var/backpack-grid-bot.sqlite'),
    serviceName: process.env.GRID_SERVICE_NAME ?? 'backpack-grid-bot'
  };
}
