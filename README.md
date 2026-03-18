# backpack-grid-bot v2

Rust rewrite of the Backpack futures grid bot.

目标：简单、高效、可靠、容错。运行目录就是仓库根目录，不再额外套 `backpack-grid-bot/` 或 `rust/` 子目录。

## 当前设计

- Rust 代码直接放在根目录 `src/`
- 根目录 `Cargo.toml`
- REST only，先把基础盘稳住
- 默认 dry-run，可切 live
- short_only 优先
- decimal 精度处理，避免脏 diff
- checkpoint + jsonl 日志
- kill switch / price range pause
- 简单同步：对比期望订单与当前挂单，补差、撤多余

## 生产上吸取的经验

- `GRID_MAX_POSITION_ABS` 是硬限制，不因为余额变大而放宽
- `short_only` 绝不允许翻成净多
- reduce-only buy 只在已有空仓时出现
- 不依赖 WS 才能安全运行
- 先保证 REST 查询 / 对账 / 下单 / 撤单链路简单可靠
- 默认安全，live 必须显式开启

## 目录

- `Cargo.toml`
- `src/`
- `deploy/backpack-grid-bot.service`
- `deploy/backpack-grid-bot.env.example`
- `deploy/install-prod.sh`

## 构建

```bash
cargo build --release
```

## 单次运行

```bash
GRID_RUN_ONCE=true cargo run --release
```

## 循环运行

```bash
GRID_RUN_ONCE=false cargo run --release
```

## 输出文件

默认写到 `runtime/`：

- `runtime/state.json`
- `runtime/events.jsonl`
- `runtime/cycles.jsonl`

## 核心环境变量

- `GRID_SYMBOL=ETH_USDC_PERP`
- `GRID_LEVELS=5`
- `GRID_SPACING_BPS=35`
- `GRID_ORDER_SIZE=0.003`
- `GRID_MAX_POSITION_ABS=0.02`
- `GRID_MODE=short_only`
- `GRID_ACTIVE_LEVELS=5`
- `GRID_MIN_PRICE=`
- `GRID_MAX_PRICE=`
- `GRID_KILL_SWITCH=false`
- `GRID_RUN_ONCE=false`
- `GRID_SHADOW_INTERVAL_MS=15000`
- `GRID_RUNTIME_STATE_PATH=runtime/state.json`
- `BACKPACK_ENABLE_LIVE=false`
- `BACKPACK_API_BASE_URL=https://api.backpack.exchange`
- `BACKPACK_API_KEY=`
- `BACKPACK_API_SECRET=`

## 说明

这版不是恢复旧 v2，而是按你最新要求重新落到仓库根目录。