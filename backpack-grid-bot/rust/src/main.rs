use anyhow::Result;
use backpack_grid_bot::{adapter::BackpackHttpClient, AppConfig, GridBotService};

fn main() -> Result<()> {
    let config = AppConfig::from_env()?;
    let client = BackpackHttpClient::from_env()?;
    let snapshot = client.fetch_snapshot(&config.symbol)?;

    let mut service = GridBotService::new(config.clone());
    let cycle = service.plan_cycle_from_snapshot(snapshot);

    println!(
        "symbol={} mark_price={} desired_orders={} cancels={} places={} matched={} synthetic_fill={}",
        config.symbol,
        cycle.state.last_mid_price.unwrap_or_default(),
        cycle.planner.desired_orders.len(),
        cycle.execution.cancel.len(),
        cycle.execution.place.len(),
        cycle.reconciliation.diff.matched,
        cycle.reconciliation.synthetic_fill.is_some()
    );

    Ok(())
}
