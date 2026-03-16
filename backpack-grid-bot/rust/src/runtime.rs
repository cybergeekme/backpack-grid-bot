use std::{thread, time::Duration};

use anyhow::Result;

use crate::{adapter::BackpackHttpClient, observability::ShadowReport, persistence::JsonFilePersistence, AppConfig, GridBotService, ServiceCycleOutput};

#[derive(Debug, Clone)]
pub struct ShadowRuntime {
    pub config: AppConfig,
    pub client: BackpackHttpClient,
    pub persistence: JsonFilePersistence,
    pub interval: Duration,
}

impl ShadowRuntime {
    pub fn from_env() -> Result<Self> {
        let config = AppConfig::from_env()?;
        let client = BackpackHttpClient::from_env()?;
        let persistence = JsonFilePersistence::from_env();
        let interval_ms = std::env::var("GRID_SHADOW_INTERVAL_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(15_000);
        Ok(Self {
            config,
            client,
            persistence,
            interval: Duration::from_millis(interval_ms),
        })
    }

    pub fn run_once(&self) -> Result<ServiceCycleOutput> {
        let snapshot = self.client.fetch_snapshot(&self.config.symbol)?;
        let mut service = GridBotService::new(self.config.clone());
        if let Some(checkpoint) = self.persistence.load_checkpoint()? {
            service.restore(checkpoint.state);
        }
        let cycle = service.plan_cycle_from_snapshot(snapshot);
        self.persistence.save_cycle(&cycle, &self.config.symbol)?;
        Ok(cycle)
    }

    pub fn run_loop(&self, iterations: Option<usize>) -> Result<()> {
        let mut remaining = iterations.unwrap_or(usize::MAX);
        while remaining > 0 {
            let cycle = self.run_once()?;
            let report = ShadowReport::from_cycle(&self.config.symbol, &cycle);
            println!(
                "shadow symbol={} mark_price={} desired={} cancels={} places={} matched={} synthetic_fill={} checkpoint={} report={}",
                self.config.symbol,
                cycle.state.last_mid_price.unwrap_or_default(),
                cycle.planner.desired_orders.len(),
                cycle.execution.cancel.len(),
                cycle.execution.place.len(),
                cycle.reconciliation.diff.matched,
                cycle.reconciliation.synthetic_fill.is_some(),
                self.persistence.path().display(),
                self.persistence.report_path().display(),
            );
            println!("{}", report.to_pretty_json()?);
            remaining = remaining.saturating_sub(1);
            if remaining > 0 {
                thread::sleep(self.interval);
            }
        }
        Ok(())
    }
}
