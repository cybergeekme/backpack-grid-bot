use anyhow::Result;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{GridLevel, OrderIntent, RuntimeEvent, RuntimeHealth, ServiceCycleOutput, SyntheticFill};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowOrderView {
    pub client_order_id: String,
    pub side: String,
    pub price: Decimal,
    pub qty: Decimal,
    pub reduce_only: bool,
    pub post_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowReport {
    pub symbol: String,
    pub mark_price: Decimal,
    pub health: RuntimeHealth,
    pub events: Vec<RuntimeEvent>,
    pub desired_orders: Vec<ShadowOrderView>,
    pub active_levels: Vec<GridLevel>,
    pub place_orders: Vec<ShadowOrderView>,
    pub cancel_orders: Vec<ShadowOrderView>,
    pub keep_orders: Vec<ShadowOrderView>,
    pub missing_on_exchange: Vec<ShadowOrderView>,
    pub unexpected_on_exchange: Vec<ShadowOrderView>,
    pub rejected_levels: Vec<(GridLevel, String)>,
    pub projected_position_after_orders: Decimal,
    pub matched: usize,
    pub synthetic_fill: Option<SyntheticFill>,
}

impl ShadowReport {
    pub fn from_cycle(symbol: impl Into<String>, cycle: &ServiceCycleOutput) -> Self {
        Self {
            symbol: symbol.into(),
            mark_price: cycle.state.last_mid_price.unwrap_or_default(),
            health: cycle.state.health.clone(),
            events: cycle.events.clone(),
            desired_orders: cycle.planner.desired_orders.iter().map(order_view_from_intent).collect(),
            active_levels: cycle.planner.active_levels.clone(),
            place_orders: cycle.execution.place.iter().map(order_view_from_intent).collect(),
            cancel_orders: cycle.execution.cancel.iter().map(|o| order_view_from_intent(&o.intent)).collect(),
            keep_orders: cycle.execution.keep.iter().map(|o| order_view_from_intent(&o.intent)).collect(),
            missing_on_exchange: cycle.reconciliation.diff.missing_on_exchange.iter().map(order_view_from_intent).collect(),
            unexpected_on_exchange: cycle
                .reconciliation
                .diff
                .unexpected_on_exchange
                .iter()
                .map(|o| order_view_from_intent(&o.intent))
                .collect(),
            rejected_levels: cycle
                .planner
                .rejected_levels
                .iter()
                .map(|(level, reason)| (level.clone(), format!("{:?}", reason)))
                .collect(),
            projected_position_after_orders: cycle.planner.projected_position_after_orders,
            matched: cycle.reconciliation.diff.matched,
            synthetic_fill: cycle.reconciliation.synthetic_fill.clone(),
        }
    }

    pub fn to_pretty_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

fn order_view_from_intent(intent: &OrderIntent) -> ShadowOrderView {
    ShadowOrderView {
        client_order_id: intent.client_order_id.clone(),
        side: format!("{:?}", intent.side).to_ascii_lowercase(),
        price: intent.price,
        qty: intent.qty,
        reduce_only: intent.reduce_only,
        post_only: intent.post_only,
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::{
        execution::ExecutionPlan, reconcile::ReconcileOutcome, service::ServiceCycleOutput, PlannerOutput, Position,
        RuntimeHealth, RuntimeState,
    };

    #[test]
    fn report_serializes_cycle_summary() {
        let cycle = ServiceCycleOutput {
            planner: PlannerOutput {
                active_levels: vec![],
                desired_orders: vec![],
                rejected_levels: vec![],
                projected_position_after_orders: dec!(-0.006),
            },
            execution: ExecutionPlan::default(),
            reconciliation: ReconcileOutcome::default(),
            events: vec![],
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
        };
        let report = ShadowReport::from_cycle("ETH_USDC_PERP", &cycle);
        let json = report.to_pretty_json().unwrap();
        assert!(json.contains("ETH_USDC_PERP"));
        assert!(json.contains("2260.4"));
    }
}
