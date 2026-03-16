use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{RuntimeState, ServiceCycleOutput};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeCheckpoint {
    pub state: RuntimeState,
    pub symbol: String,
    pub desired_orders: usize,
    pub matched_orders: usize,
    pub synthetic_fill: bool,
    pub saved_at_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub struct JsonFilePersistence {
    path: PathBuf,
}

impl JsonFilePersistence {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
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

    pub fn path(&self) -> &PathBuf {
        &self.path
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
        reconcile::ReconcileOutcome, service::ServiceCycleOutput, ExecutionPlan, PlannerOutput, Position, RuntimeState,
    };

    #[test]
    fn saves_and_loads_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonFilePersistence::new(dir.path().join("state.json"));
        let cycle = ServiceCycleOutput {
            planner: PlannerOutput {
                active_levels: vec![],
                desired_orders: vec![],
                rejected_levels: vec![],
                projected_position_after_orders: dec!(0),
            },
            execution: ExecutionPlan::default(),
            reconciliation: ReconcileOutcome::default(),
            state: RuntimeState {
                working_orders: vec![],
                recent_fills: vec![],
                position: Some(Position {
                    symbol: "ETH_USDC_PERP".into(),
                    size: dec!(-0.003),
                    entry_price: dec!(2255.45),
                    unrealized_pnl: dec!(0),
                }),
                last_mid_price: Some(dec!(2260.4)),
            },
        };

        store.save_cycle(&cycle, "ETH_USDC_PERP").unwrap();
        let loaded = store.load_checkpoint().unwrap().unwrap();
        assert_eq!(loaded.symbol, "ETH_USDC_PERP");
        assert_eq!(loaded.state.position.unwrap().size, dec!(-0.003));
    }
}
