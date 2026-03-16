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
        let report = ShadowReport::from_cycle(&self.config.symbol, &cycle);
        self.persistence.save_report(&report)?;
        self.persistence.append_events(&cycle, &self.config.symbol)?;
        self.persistence.append_cycle_summary(&cycle, &self.config.symbol)?;
        Ok(cycle)
    }

    pub fn run_loop(&self, iterations: Option<usize>) -> Result<()> {
        let mut remaining = iterations.unwrap_or(usize::MAX);
        while remaining > 0 {
            let cycle = self.run_once()?;
            let report = ShadowReport::from_cycle(&self.config.symbol, &cycle);
            println!(
                "shadow symbol={} mark_price={} state={:?} pause_reason={:?} issues={:?} events={} desired={} cancels={} places={} executed_mode={} executed_places={} executed_cancels={} executed_final={} projection_matched={} projection_only={} exchange_only={} matched={} synthetic_fill={} checkpoint={} report={} journal={} cycles={}",
                self.config.symbol,
                cycle.state.last_mid_price.unwrap_or_default(),
                cycle.state.health.service_state,
                cycle.state.health.pause_reason,
                cycle.state.health.issues,
                cycle.events.len(),
                cycle.planner.desired_orders.len(),
                cycle.execution.cancel.len(),
                cycle.execution.place.len(),
                report.execution_summary.mode.clone().unwrap_or_else(|| "none".to_string()),
                report.execution_summary.placed_count,
                report.execution_summary.cancelled_count,
                report.execution_summary.final_order_count,
                report.projection_summary.matched_count,
                report.projection_summary.only_in_projection_count,
                report.projection_summary.only_on_exchange_count,
                cycle.reconciliation.diff.matched,
                cycle.reconciliation.synthetic_fill.is_some(),
                self.persistence.path().display(),
                self.persistence.report_path().display(),
                self.persistence.event_journal_path().display(),
                self.persistence.cycle_summary_path().display(),
            );
            if let Some(message) = report.event_summary.latest_messages.first() {
                println!("shadow latest_event={}", message);
            }
            println!("{}", report.to_pretty_json()?);
            remaining = remaining.saturating_sub(1);
            if remaining > 0 {
                thread::sleep(self.interval);
            }
        }
        Ok(())
    }
}
