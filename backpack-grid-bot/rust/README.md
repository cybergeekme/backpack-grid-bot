# Rust rewrite bootstrap

This directory is the start of the Rust-first rewrite for `backpack-grid-bot`.

## Current scope

Implemented skeleton modules:

- `domain.rs`
- `config.rs`
- `precision.rs`
- `risk.rs`
- `planner.rs`
- `main.rs`

## Design intent

- Real trading core uses `rust_decimal`, not `f64`
- `short_only` behavior is preserved as a first-class rule
- Position projection and order-qty normalization are centralized
- Planner and risk logic are separated from exchange I/O
- Existing TypeScript code remains the behavior reference, not the runtime target

## Next modules to build

1. `adapter/backpack.rs`
2. `execution.rs`
3. `reconcile.rs`
4. `alerts.rs`
5. `state.rs`

## Notes

Rust toolchain was not present on this machine at bootstrap time (`cargo` / `rustc` missing), so this skeleton was created without local compilation.
