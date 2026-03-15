# Backpack Futures Grid v1 Skeleton

A production-oriented TypeScript skeleton for a futures grid trading service.

## What it includes

- Exchange adapter contract
- Safe Backpack adapter stub (REST + WebSocket scaffolding)
- Typed Backpack WebSocket client for public/private subscription handling
- Mock exchange adapter for local simulation
- Risk engine
- Grid strategy engine
- OMS / order manager
- Basic REST-vs-desired order reconciliation
- SQLite persistence with WAL mode and schema initialization
- Restart recovery for strategy snapshot, working orders, recent fills, position, and service checkpoints
- Mock/demo adapter checkpoint restore so local restarts keep simulated state
- Runnable mock demo simulation
- Real long-running `service` mode with conservative safety states
- Basic tests
- systemd deployment example

## Safety

- The demo and service both use only a mock exchange by default.
- Backpack live mode is opt-in and hard-disabled unless `BACKPACK_ENABLE_LIVE=true` is set.
- Service mode has explicit health states: `STARTING`, `ACTIVE`, `DEGRADED`, `PAUSED`, `SAFE_MODE`, `STOPPED`.
- Out-of-range price moves pause grid placement instead of blindly chasing price.
- Consecutive loop failures trip a circuit breaker into `SAFE_MODE`; reconciliation continues but new rebalances stop.
- Periodic REST reconciliation refreshes orders/position even while paused or degraded.
- Optional Telegram alerting can page operators for lifecycle/state changes, circuit breaker trips, out-of-range pauses, reconciliation mismatches, and fills.
- WebSocket connection attempts inherit the same live-trading safety gate and degrade back to REST-only if WS connect fails.
- Backpack integration currently covers authenticated REST calls for balances, open orders, single-position lookup, order placement/cancel, public mark prices, and typed WS normalization scaffolding for order/position/fill updates.
- Persistence now covers local runtime snapshots and checkpoints, but live-trading replay logic after partial exchange disconnects is still incomplete.

## Quick start

```bash
npm install
npm run test
npm run build
npm run demo
GRID_SERVICE_DRY_RUN_MS=45000 npm run service
```

## Environment

Use `.env.example` for local development, and `deploy/backpack-grid-bot.env.example` as the template for `/etc/backpack-grid-bot/backpack-grid-bot.env` in production.

Optional:

- `GRID_SYMBOL` (default: `ETH_USDC_PERP`)
- `GRID_LEVELS` (default: `3`)
- `GRID_SPACING_BPS` (default: `50`)
- `GRID_ORDER_SIZE` (default: `0.01`)
- `GRID_MAX_POSITION_ABS` (default: `0.05`)
- `GRID_KILL_SWITCH=true` to force strategy shutdown
- `GRID_USE_BACKPACK=true` to switch from mock exchange to Backpack REST adapter
- `GRID_DB_PATH=./var/backpack-grid-bot.sqlite` SQLite database path for persistent state
- `GRID_SERVICE_NAME=backpack-grid-bot` logical service name used to partition persisted rows
- `GRID_SERVICE_LOOP_MS=15000` main service loop interval
- `GRID_SERVICE_RECONCILE_MS=60000` periodic REST reconciliation interval
- `GRID_SERVICE_ERROR_THRESHOLD=3` consecutive loop errors before SAFE_MODE
- `GRID_SERVICE_OUT_OF_RANGE_BPS=75` pause buffer beyond the last active grid before new orders are suppressed
- `GRID_SERVICE_DRY_RUN_MS=0` optional auto-stop timer for service dry runs
- `GRID_MOCK_PRICE_STEP=100` mock-service price step used only in mock service mode
- `TELEGRAM_ALERTS_ENABLED=true` enable Telegram operator alerts
- `TELEGRAM_BOT_TOKEN=<bot token>` Telegram Bot API token
- `TELEGRAM_CHAT_ID=<chat id>` Telegram target chat/channel/user id
- `TELEGRAM_ALERT_LEVEL=warn` minimum severity to send: `info`, `warn`, `critical`
- `TELEGRAM_ALERT_DEDUP_MS=300000` per-alert dedup window in milliseconds
- `TELEGRAM_NOTIFY_FILLS=false` set true to send fill notifications in addition to safety alerts
- `BACKPACK_ENABLE_LIVE=true` to actually allow authenticated Backpack calls
- `BACKPACK_ENABLE_WS=true` to enable the Backpack WS client when live mode is enabled (default: true)
- `BACKPACK_API_KEY=<base64 public key>`
- `BACKPACK_API_SECRET=<base64 ed25519 seed>`
- `BACKPACK_WINDOW_MS=5000`
- `BACKPACK_WS_URL=wss://ws.backpack.exchange` to override the default endpoint while testing

## Persistence model

The service now stores the following in SQLite:

- strategy snapshots
- working/open order state
- fills
- reconciliation events
- risk rejection events
- service checkpoints, including a runtime snapshot payload

On startup, the service loads the latest persisted runtime checkpoint and restores:

- latest strategy snapshot
- latest position
- latest working orders
- recent fills
- mock adapter internals in demo mode

SQLite is opened in WAL mode with `synchronous=NORMAL` for a decent durability/latency trade-off on a single-host service.

## Deployment with systemd

Example unit: `deploy/backpack-grid-bot.service`

Suggested layout on Ubuntu 24.04:

```bash
sudo useradd --system --home /opt/backpack-grid-bot --shell /usr/sbin/nologin backpack-grid-bot
sudo mkdir -p /opt/backpack-grid-bot /etc/backpack-grid-bot /var/lib/backpack-grid-bot
sudo chown -R backpack-grid-bot:backpack-grid-bot /opt/backpack-grid-bot /var/lib/backpack-grid-bot

# copy project files, then:
cd /opt/backpack-grid-bot
npm ci
npm run build
GRID_SERVICE_DRY_RUN_MS=45000 npm run service   # optional dry run before enabling the unit

sudo cp deploy/backpack-grid-bot.service /etc/systemd/system/backpack-grid-bot.service
sudo cp deploy/backpack-grid-bot.env.example /etc/backpack-grid-bot/backpack-grid-bot.env
sudo chmod 600 /etc/backpack-grid-bot/backpack-grid-bot.env
sudo ${EDITOR:-vi} /etc/backpack-grid-bot/backpack-grid-bot.env

sudo systemctl daemon-reload
sudo systemctl enable --now backpack-grid-bot.service
sudo systemctl status backpack-grid-bot.service
```

Notes:

- The included unit uses `StateDirectory=backpack-grid-bot` and writes the SQLite DB under `/var/lib/backpack-grid-bot`.
- The `service` entrypoint is the intended long-running mode; use `GRID_SERVICE_DRY_RUN_MS` for supervised smoke tests.
- Telegram alert delivery is fire-and-forget; send failures are logged but do not stop the trading loop.
- If `TELEGRAM_ALERTS_ENABLED=true` but token/chat id are missing or wrong, alerts will be skipped or logged as delivery failures rather than crashing the service.
- If you enable live mode, keep the env file readable only by root and the service account.
- Treat `SAFE_MODE` and repeated `PAUSED` logs as operator-review conditions, not something to auto-ignore.

## Backpack notes

This project follows Backpack's ED25519 signing model and current REST/WS surface documented at <https://docs.backpack.exchange/>. The adapter currently targets:

- `GET /api/v1/time`
- `GET /api/v1/markPrices`
- `GET /api/v1/capital`
- `GET /api/v1/orders`
- `GET /api/v1/position`
- `POST /api/v1/order`
- `DELETE /api/v1/order`
- WebSocket subscribe scaffolding for mark prices, orders, and positions

Before using with real funds, you should still add:

- validated Backpack WS auth/subscribe payloads against a live account capture
- reconnect/backoff and ping/pong supervision
- sequence-aware replay + REST catch-up after disconnects
- stronger response validation against live payloads
- additional alert sinks (Discord/PagerDuty/etc.) if Telegram alone is not enough for operations
- cancel-on-pause / flatten-on-safe-mode policies after live validation proves the right behavior
- sandbox/small-size validation against your Backpack subaccount
