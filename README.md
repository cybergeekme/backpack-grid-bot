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
- Runnable demo simulation
- Basic tests
- systemd deployment example

## Safety

- The demo uses only a mock exchange by default.
- Backpack live mode is opt-in and hard-disabled unless `BACKPACK_ENABLE_LIVE=true` is set.
- WebSocket connection attempts inherit the same live-trading safety gate and degrade back to REST-only if WS connect fails.
- Backpack integration currently covers authenticated REST calls for balances, open orders, single-position lookup, order placement/cancel, public mark prices, and typed WS normalization scaffolding for order/position/fill updates.
- Persistence now covers local runtime snapshots and checkpoints, but live-trading replay logic after partial exchange disconnects is still incomplete.

## Quick start

```bash
npm install
npm run test
npm run build
npm run demo
```

## Environment

Optional:

- `GRID_SYMBOL` (default: `BTC_USDC_PERP`)
- `GRID_LEVELS` (default: `3`)
- `GRID_SPACING_BPS` (default: `50`)
- `GRID_ORDER_SIZE` (default: `0.01`)
- `GRID_MAX_POSITION_ABS` (default: `0.05`)
- `GRID_KILL_SWITCH=true` to force strategy shutdown
- `GRID_USE_BACKPACK=true` to switch from mock exchange to Backpack REST adapter
- `GRID_DB_PATH=./var/backpack-grid-bot.sqlite` SQLite database path for persistent state
- `GRID_SERVICE_NAME=backpack-grid-bot` logical service name used to partition persisted rows
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

sudo cp deploy/backpack-grid-bot.service /etc/systemd/system/backpack-grid-bot.service
sudo tee /etc/backpack-grid-bot/backpack-grid-bot.env >/dev/null <<'EOF'
GRID_SYMBOL=BTC_USDC_PERP
GRID_LEVELS=3
GRID_SPACING_BPS=50
GRID_ORDER_SIZE=0.01
GRID_MAX_POSITION_ABS=0.05
GRID_DB_PATH=/var/lib/backpack-grid-bot/backpack-grid-bot.sqlite
GRID_SERVICE_NAME=backpack-grid-bot-prod
# GRID_USE_BACKPACK=true
# BACKPACK_ENABLE_LIVE=true
# BACKPACK_API_KEY=...
# BACKPACK_API_SECRET=...
EOF

sudo systemctl daemon-reload
sudo systemctl enable --now backpack-grid-bot.service
sudo systemctl status backpack-grid-bot.service
```

Notes:

- The included unit uses `StateDirectory=backpack-grid-bot` and writes the SQLite DB under `/var/lib/backpack-grid-bot`.
- Replace the demo `ExecStart` command with your production entrypoint/loop once you add a long-running scheduler around `rebalance()`.
- If you enable live mode, keep the env file readable only by root and the service account.

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
- a real long-running scheduling/worker loop instead of the current demo entrypoint
- stronger response validation against live payloads
- sandbox/small-size validation against your Backpack subaccount
