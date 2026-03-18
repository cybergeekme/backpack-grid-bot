# backpack-grid-bot v2

Rust rewrite of the Backpack futures grid bot.

Goal: simple, production-usable, short-only first, dry-run by default, with the safety lessons from the TypeScript production validation carried over.

## What this version is

- Rust-first runtime under `rust/`
- read-only / shadow mode by default
- deterministic grid planning with decimal math
- restart checkpoint persistence
- append-only event and cycle journals
- reconciliation-based mismatch detection
- synthetic fill inference when exchange events are missing
- kill-switch and configured price-range pause guards
- short-only protection against flipping net long

## What this version is not

- not a sprawling framework
- not dependent on WebSocket correctness to stay safe
- not live by default

The TS implementation remains the behavior reference in git history. The v2 branch runtime target is the Rust service.

## Production behaviors carried over from prior incidents

- `short_only` is for a clean account context or an already-short account, not for adopting an oversized unrelated short.
- `GRID_MAX_POSITION_ABS` is the hard cap. More balance does not override it.
- reduce-only buys are only valid when a short already exists.
- order / position drift must be visible in journals and reports instead of being silently ignored.
- missing exchange fill events are handled by reconciliation fallback inference.
- decimal normalization is mandatory so tiny precision noise does not create bogus diffs.
- the process must fail safe: dry-run first, explicit opt-in before any future live mutation path.

## Layout

- `rust/` — Rust runtime
- `deploy/backpack-grid-bot.service` — systemd unit
- `deploy/backpack-grid-bot.env.example` — production env template
- `deploy/install-prod.sh` — install/update helper

## Build

```bash
cd rust
cargo test
cargo build --release
```

## Run once

```bash
cd rust
GRID_RUN_ONCE=true cargo run --release
```

## Run loop

```bash
cd rust
GRID_RUN_ONCE=false cargo run --release
```

Default output is written under `runtime/` unless overridden:

- `runtime/state.json`
- `runtime/shadow-report.json`
- `runtime/events.jsonl`
- `runtime/cycles.jsonl`

## Required env

- `GRID_SYMBOL` default `ETH_USDC_PERP`
- `GRID_LEVELS`
- `GRID_SPACING_BPS`
- `GRID_ORDER_SIZE`
- `GRID_MAX_POSITION_ABS`
- `GRID_MODE=short_only` recommended for production
- `GRID_ACTIVE_LEVELS`
- `GRID_MIN_PRICE` / `GRID_MAX_PRICE` optional hard bounds
- `GRID_KILL_SWITCH=true` to force pause
- `BACKPACK_API_KEY`
- `BACKPACK_API_SECRET`
- `BACKPACK_API_BASE_URL` optional, default `https://api.backpack.exchange`
- `GRID_RUNTIME_STATE_PATH` optional, default `runtime/state.json`
- `GRID_SHADOW_INTERVAL_MS` optional, default `15000`

## Production flow

1. configure env
2. run once in read-only mode and inspect `shadow-report.json`
3. run the service under systemd
4. watch `events.jsonl` and `cycles.jsonl`
5. only after repeated clean shadow cycles should you consider adding a real execution adapter

## Current execution mode

This branch currently executes in `DryRun` mode only. That is intentional: production-safe observation first, mutation later only after explicit validation.

If you want, the next step after this branch is to wire a live execution adapter behind an explicit safety gate instead of replacing the current safe default.
