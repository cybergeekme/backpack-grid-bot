import { BackpackFuturesAdapter } from './adapters/backpackAdapter';
import { MockExchangeAdapter } from './adapters/mockExchangeAdapter';
import { loadConfig } from './config';
import { GridTradingService } from './service/gridService';
import { ServiceRunner } from './service/serviceRunner';

function createAdapter(config: ReturnType<typeof loadConfig>) {
  return config.useBackpack
    ? new BackpackFuturesAdapter({
        apiKey: config.backpackApiKey,
        apiSecret: config.backpackApiSecret,
        liveEnabled: config.backpackLiveEnabled,
        recvWindowMs: config.backpackWindowMs,
        enableWebSocket: config.backpackWsEnabled,
        symbol: config.symbol
      })
    : new MockExchangeAdapter(100_000, config.symbol);
}

async function demo(): Promise<void> {
  const config = loadConfig();
  const adapter = createAdapter(config);
  const service = new GridTradingService(config, adapter);
  await service.start();
  await service.rebalance();

  if (adapter instanceof MockExchangeAdapter) {
    for (const price of [99_700, 100_300, 99_200, 100_800]) {
      adapter.movePrice(price);
      await service.rebalance();
    }
    console.log(JSON.stringify({ demo: 'complete', state: service.snapshot(), fills: adapter.getFills() }, null, 2));
  }

  await service.stop();
}

async function serviceCommand(): Promise<void> {
  const config = loadConfig();
  const adapter = createAdapter(config);
  const service = new GridTradingService(config, adapter);
  let tickDirection = 1;
  const runner = new ServiceRunner(config, service, {
    onMockTick: adapter instanceof MockExchangeAdapter
      ? () => {
          const current = service.getRuntimeHealth().lastMidPrice ?? 100_000;
          const next = current + config.serviceMockPriceStep * tickDirection;
          if (Math.abs(next - 100_000) >= config.serviceMockPriceStep * 3) tickDirection *= -1;
          adapter.movePrice(next);
        }
      : undefined
  });

  const shutdown = async (signal: string) => {
    console.log(JSON.stringify({ ts: new Date().toISOString(), level: 'info', scope: 'CLI', message: 'Shutdown requested.', signal }));
    await runner.stop(signal);
  };

  process.once('SIGINT', () => void shutdown('SIGINT'));
  process.once('SIGTERM', () => void shutdown('SIGTERM'));

  const dryRunMs = Number(process.env.GRID_SERVICE_DRY_RUN_MS ?? 0);
  if (dryRunMs > 0) {
    setTimeout(() => {
      void shutdown(`dry_run_${dryRunMs}ms`);
    }, dryRunMs).unref();
  }

  await runner.start();
}

async function main(): Promise<void> {
  const command = process.argv[2] ?? 'demo';
  if (command === 'demo') {
    await demo();
    return;
  }
  if (command === 'service') {
    await serviceCommand();
    return;
  }
  throw new Error(`Unknown command: ${command}`);
}

main().catch((error: unknown) => {
  const message = error instanceof Error ? error.stack ?? error.message : String(error);
  console.error(message);
  process.exitCode = 1;
});
