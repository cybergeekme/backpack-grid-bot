import { BackpackFuturesAdapter } from './adapters/backpackAdapter';
import { MockExchangeAdapter } from './adapters/mockExchangeAdapter';
import { loadConfig } from './config';
import { GridTradingService } from './service/gridService';

async function demo(): Promise<void> {
  const config = loadConfig();
  const adapter = config.useBackpack
    ? new BackpackFuturesAdapter({
        apiKey: config.backpackApiKey,
        apiSecret: config.backpackApiSecret,
        liveEnabled: config.backpackLiveEnabled,
        recvWindowMs: config.backpackWindowMs,
        enableWebSocket: config.backpackWsEnabled,
        symbol: config.symbol
      })
    : new MockExchangeAdapter(100_000, config.symbol);

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

async function main(): Promise<void> {
  const command = process.argv[2] ?? 'demo';
  if (command === 'demo') {
    await demo();
    return;
  }
  throw new Error(`Unknown command: ${command}`);
}

main().catch((error: unknown) => {
  const message = error instanceof Error ? error.stack ?? error.message : String(error);
  console.error(message);
  process.exitCode = 1;
});
