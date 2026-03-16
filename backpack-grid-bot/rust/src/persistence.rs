use std::{fs, io::Write, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{observability::ShadowReport, RuntimeEvent, RuntimeState, ServiceCycleOutput};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeCheckpoint {
    pub state: RuntimeState,
    pub symbol: String,
    pub desired_orders: usize,
    pub matched_orders: usize,
    pub synthetic_fill: bool,
    pub saved_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventJournalRecord {
    pub symbol: String,
    pub recorded_at_unix_ms: i64,
    pub mark_price: String,
    pub service_state: String,
    pub pause_reason: Option<String>,
    pub event: RuntimeEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CycleSummaryRecord {
    pub symbol: String,
    pub recorded_at_unix_ms: i64,
    pub mark_price: String,
    pub service_state: String,
    pub pause_reason: Option<String>,
    pub issue_count: usize,
    pub event_count: usize,
    pub desired_orders: usize,
    pub place_orders: usize,
    pub cancel_orders: usize,
    pub keep_orders: usize,
    pub matched_orders: usize,
    pub synthetic_fill: bool,
}

#[derive(Debug, Clone)]
pub struct JsonFilePersistence {
    path: PathBuf,
    report_path: PathBuf,
    event_journal_path: PathBuf,
    cycle_summary_path: PathBuf,
}

impl JsonFilePersistence {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let parent = path.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let report_path = parent.join("shadow-report.json");
        let event_journal_path = parent.join("events.jsonl");
        let cycle_summary_path = parent.join("cycles.jsonl");
        Self {
            path,
            report_path,
            event_journal_path,
            cycle_summary_path,
        }
    }

    pub fn from_env() -> Self {
        let path = std::env::var("GRID_RUNTIME_STATE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("runtime/state.json"));
        Self::new(path)
    }

    pub fn save_cycle(&self, cycle: &ServiceCycleOutput, symbol: &str) -> Result<()> {
        let checkpoint = RuntimeCheckpoint {
            state: cycle.state.clone(),
            symbol: symbol.to_string(),
            desired_orders: cycle.planner.desired_orders.len(),
            matched_orders: cycle.reconciliation.diff.matched,
            synthetic_fill: cycle.reconciliation.synthetic_fill.is_some(),
            saved_at_unix_ms: now_unix_ms(),
        };
        self.save_checkpoint(&checkpoint)
    }

    pub fn save_checkpoint(&self, checkpoint: &RuntimeCheckpoint) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create checkpoint dir {}", parent.display()))?;
        }
        let body = serde_json::to_string_pretty(checkpoint)?;
        fs::write(&self.path, body).with_context(|| format!("write checkpoint {}", self.path.display()))?;
        Ok(())
    }

    pub fn load_checkpoint(&self) -> Result<Option<RuntimeCheckpoint>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let body = fs::read_to_string(&self.path).with_context(|| format!("read checkpoint {}", self.path.display()))?;
        let checkpoint = serde_json::from_str::<RuntimeCheckpoint>(&body)
            .with_context(|| format!("parse checkpoint {}", self.path.display()))?;
        Ok(Some(checkpoint))
    }

    pub fn save_report(&self, report: &ShadowReport) -> Result<()> {
        if let Some(parent) = self.report_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create report dir {}", parent.display()))?;
        }
        let body = report.to_pretty_json()?;
        fs::write(&self.report_path, body).with_context(|| format!("write report {}", self.report_path.display()))?;
        Ok(())
    }

    pub fn append_events(&self, cycle: &ServiceCycleOutput, symbol: &str) -> Result<usize> {
        if cycle.events.is_empty() {
            return Ok(0);
        }
        if let Some(parent) = self.event_journal_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create event journal dir {}", parent.display()))?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.event_journal_path)
            .with_context(|| format!("open event journal {}", self.event_journal_path.display()))?;
        let recorded_at_unix_ms = now_unix_ms();
        let mark_price = cycle.state.last_mid_price.unwrap_or_default().to_string();
        let service_state = format!("{:?}", cycle.state.health.service_state);
        let pause_reason = cycle.state.health.pause_reason.map(|reason| format!("{:?}", reason));

        for event in &cycle.events {
            let record = EventJournalRecord {
                symbol: symbol.to_string(),
                recorded_at_unix_ms,
                mark_price: mark_price.clone(),
                service_state: service_state.clone(),
                pause_reason: pause_reason.clone(),
                event: event.clone(),
            };
            let line = serde_json::to_string(&record)?;
            writeln!(file, "{line}")?;
        }
        Ok(cycle.events.len())
    }

    pub fn append_cycle_summary(&self, cycle: &ServiceCycleOutput, symbol: &str) -> Result<()> {
        if let Some(parent) = self.cycle_summary_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create cycle summary dir {}", parent.display()))?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.cycle_summary_path)
            .with_context(|| format!("open cycle summary {}", self.cycle_summary_path.display()))?;
        let record = CycleSummaryRecord {
            symbol: symbol.to_string(),
            recorded_at_unix_ms: now_unix_ms(),
            mark_price: cycle.state.last_mid_price.unwrap_or_default().to_string(),
            service_state: format!("{:?}", cycle.state.health.service_state),
            pause_reason: cycle.state.health.pause_reason.map(|reason| format!("{:?}", reason)),
            issue_count: cycle.state.health.issues.len(),
            event_count: cycle.events.len(),
            desired_orders: cycle.planner.desired_orders.len(),
            place_orders: cycle.execution.place.len(),
            cancel_orders: cycle.execution.cancel.len(),
            keep_orders: cycle.execution.keep.len(),
            matched_orders: cycle.reconciliation.diff.matched,
            synthetic_fill: cycle.reconciliation.synthetic_fill.is_some(),
        };
        let line = serde_json::to_string(&record)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn report_path(&self) -> &PathBuf {
        &self.report_path
    }

    pub fn event_journal_path(&self) -> &PathBuf {
        &self.event_journal_path
    }

    pub fn cycle_summary_path(&self) -> &PathBuf {
        &self.cycle_summary_path
    }
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::{
        reconcile::ReconcileOutcome, service::ServiceCycleOutput, ExecutionPlan, PlannerOutput, Position, RuntimeHealth,
        RuntimeState,
    };

    fn sample_cycle() -> ServiceCycleOutput {
        ServiceCycleOutput {
            planner: PlannerOutput {
                active_levels: vec![],
                desired_orders: vec![],
                rejected_levels: vec![],
                projected_position_after_orders: dec!(0),
            },
            execution: ExecutionPlan::default(),
            executed: None,
            reconciliation: ReconcileOutcome::default(),
            events: vec![crate::RuntimeEvent::service_state_changed(crate::ServiceState::Active, "active")],
            state: RuntimeState {
                working_orders: vec![],
                recent_fills: vec![],
                recent_events: vec![],
                position: Some(Position {
                    symbol: "ETH_USDC_PERP".into(),
                    size: dec!(-0.003),
                    entry_price: dec!(2255.45),
                    unrealized_pnl: dec!(0),
                }),
                last_mid_price: Some(dec!(2260.4)),
                health: RuntimeHealth::default(),
            },
        }
    }

    #[test]
    fn saves_and_loads_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonFilePersistence::new(dir.path().join("state.json"));
        let cycle = sample_cycle();

        store.save_cycle(&cycle, "ETH_USDC_PERP").unwrap();
        let loaded = store.load_checkpoint().unwrap().unwrap();
        assert_eq!(loaded.symbol, "ETH_USDC_PERP");
        assert_eq!(loaded.state.position.unwrap().size, dec!(-0.003));
    }

    #[test]
    fn appends_event_journal_records() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonFilePersistence::new(dir.path().join("state.json"));
        let cycle = sample_cycle();

        let written = store.append_events(&cycle, "ETH_USDC_PERP").unwrap();
        assert_eq!(written, 1);

        let body = fs::read_to_string(store.event_journal_path()).unwrap();
        assert!(body.contains("ETH_USDC_PERP"));
        assert!(body.contains("service_state_changed"));
    }

    #[test]
    fn appends_cycle_summary_records() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonFilePersistence::new(dir.path().join("state.json"));
        let cycle = sample_cycle();

        store.append_cycle_summary(&cycle, "ETH_USDC_PERP").unwrap();
        let body = fs::read_to_string(store.cycle_summary_path()).unwrap();
        assert!(body.contains("ETH_USDC_PERP"));
        assert!(body.contains("event_count"));
        assert!(body.contains("service_state"));
    }
}
