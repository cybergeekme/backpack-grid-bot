use anyhow::Result;
use backpack_grid_bot::ShadowRuntime;

fn main() -> Result<()> {
    let runtime = ShadowRuntime::from_env()?;
    let once = std::env::var("GRID_RUN_ONCE")
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(true);

    if once {
        let cycle = runtime.run_once()?;
        println!(
            "shadow symbol={} mark_price={} desired_orders={} cancels={} places={} matched={} synthetic_fill={} checkpoint={}",
            runtime.config.symbol,
            cycle.state.last_mid_price.unwrap_or_default(),
            cycle.planner.desired_orders.len(),
            cycle.execution.cancel.len(),
            cycle.execution.place.len(),
            cycle.reconciliation.diff.matched,
            cycle.reconciliation.synthetic_fill.is_some(),
            runtime.persistence.path().display(),
        );
        return Ok(());
    }

    runtime.run_loop(None)
}
