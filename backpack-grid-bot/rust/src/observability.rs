use anyhow::Result;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{ExecutionMode, GridLevel, OrderIntent, RuntimeEvent, RuntimeHealth, ServiceCycleOutput, SyntheticFill};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct OrderCompareKey {
    side: String,
    price: String,
    qty: String,
    reduce_only: bool,
    post_only: bool,
}

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
pub struct ShadowEventSummary {
    pub event_count: usize,
    pub latest_messages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowHealthSummary {
    pub service_state: String,
    pub pause_reason: Option<String>,
    pub issues: Vec<String>,
    pub consecutive_errors: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowExecutionSummary {
    pub mode: Option<String>,
    pub placed_count: usize,
    pub cancelled_count: usize,
    pub retained_count: usize,
    pub final_order_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowProjectionSummary {
    pub actual_open_order_count: usize,
    pub projected_final_order_count: usize,
    pub matched_count: usize,
    pub only_on_exchange_count: usize,
    pub only_in_projection_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowReport {
    pub symbol: String,
    pub mark_price: Decimal,
    pub health: RuntimeHealth,
    pub health_summary: ShadowHealthSummary,
    pub events: Vec<RuntimeEvent>,
    pub event_summary: ShadowEventSummary,
    pub execution_summary: ShadowExecutionSummary,
    pub projection_summary: ShadowProjectionSummary,
    pub desired_orders: Vec<ShadowOrderView>,
    pub active_levels: Vec<GridLevel>,
    pub place_orders: Vec<ShadowOrderView>,
    pub cancel_orders: Vec<ShadowOrderView>,
    pub keep_orders: Vec<ShadowOrderView>,
    pub executed_place_orders: Vec<ShadowOrderView>,
    pub executed_cancel_orders: Vec<ShadowOrderView>,
    pub executed_keep_orders: Vec<ShadowOrderView>,
    pub executed_final_orders: Vec<ShadowOrderView>,
    pub actual_open_orders: Vec<ShadowOrderView>,
    pub projection_only_orders: Vec<ShadowOrderView>,
    pub exchange_only_orders: Vec<ShadowOrderView>,
    pub missing_on_exchange: Vec<ShadowOrderView>,
    pub unexpected_on_exchange: Vec<ShadowOrderView>,
    pub rejected_levels: Vec<(GridLevel, String)>,
    pub projected_position_after_orders: Decimal,
    pub matched: usize,
    pub synthetic_fill: Option<SyntheticFill>,
}

impl ShadowReport {
    pub fn from_cycle(symbol: impl Into<String>, cycle: &ServiceCycleOutput) -> Self {
        let executed_final_orders: Vec<ShadowOrderView> = cycle
            .executed
            .as_ref()
            .map(|executed| {
                executed
                    .final_orders
                    .iter()
                    .map(|o| order_view_from_intent(&o.intent))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let actual_open_orders = cycle
            .state
            .working_orders
            .iter()
            .map(|o| order_view_from_intent(&o.intent))
            .collect::<Vec<_>>();
        let (projection_only_orders, exchange_only_orders, matched_count) = compare_orders(&executed_final_orders, &actual_open_orders);

        Self {
            symbol: symbol.into(),
            mark_price: cycle.state.last_mid_price.unwrap_or_default(),
            health: cycle.state.health.clone(),
            health_summary: ShadowHealthSummary {
                service_state: format!("{:?}", cycle.state.health.service_state),
                pause_reason: cycle.state.health.pause_reason.map(|reason| format!("{:?}", reason)),
                issues: cycle.state.health.issues.iter().map(|issue| format!("{:?}", issue)).collect(),
                consecutive_errors: cycle.state.health.consecutive_errors,
            },
            events: cycle.events.clone(),
            event_summary: ShadowEventSummary {
                event_count: cycle.events.len(),
                latest_messages: cycle.events.iter().take(5).map(|event| event.message.clone()).collect(),
            },
            execution_summary: ShadowExecutionSummary {
                mode: cycle.executed.as_ref().map(|executed| execution_mode_label(&executed.mode)),
                placed_count: cycle.executed.as_ref().map(|executed| executed.placed.len()).unwrap_or(0),
                cancelled_count: cycle.executed.as_ref().map(|executed| executed.cancelled.len()).unwrap_or(0),
                retained_count: cycle.executed.as_ref().map(|executed| executed.retained.len()).unwrap_or(0),
                final_order_count: cycle.executed.as_ref().map(|executed| executed.final_orders.len()).unwrap_or(0),
            },
            projection_summary: ShadowProjectionSummary {
                actual_open_order_count: actual_open_orders.len(),
                projected_final_order_count: executed_final_orders.len(),
                matched_count,
                only_on_exchange_count: exchange_only_orders.len(),
                only_in_projection_count: projection_only_orders.len(),
            },
            desired_orders: cycle.planner.desired_orders.iter().map(order_view_from_intent).collect(),
            active_levels: cycle.planner.active_levels.clone(),
            place_orders: cycle.execution.place.iter().map(order_view_from_intent).collect(),
            cancel_orders: cycle.execution.cancel.iter().map(|o| order_view_from_intent(&o.intent)).collect(),
            keep_orders: cycle.execution.keep.iter().map(|o| order_view_from_intent(&o.intent)).collect(),
            executed_place_orders: cycle
                .executed
                .as_ref()
                .map(|executed| executed.placed.iter().map(|o| order_view_from_intent(&o.intent)).collect())
                .unwrap_or_default(),
            executed_cancel_orders: cycle
                .executed
                .as_ref()
                .map(|executed| executed.cancelled.iter().map(|o| order_view_from_intent(&o.intent)).collect())
                .unwrap_or_default(),
            executed_keep_orders: cycle
                .executed
                .as_ref()
                .map(|executed| executed.retained.iter().map(|o| order_view_from_intent(&o.intent)).collect())
                .unwrap_or_default(),
            executed_final_orders,
            actual_open_orders,
            projection_only_orders,
            exchange_only_orders,
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

fn execution_mode_label(mode: &ExecutionMode) -> String {
    match mode {
        ExecutionMode::DryRun => "dry_run".to_string(),
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

fn compare_orders(projected: &[ShadowOrderView], actual: &[ShadowOrderView]) -> (Vec<ShadowOrderView>, Vec<ShadowOrderView>, usize) {
    let projected_map = projected
        .iter()
        .cloned()
        .map(|order| (compare_key(&order), order))
        .collect::<std::collections::BTreeMap<_, _>>();
    let actual_map = actual
        .iter()
        .cloned()
        .map(|order| (compare_key(&order), order))
        .collect::<std::collections::BTreeMap<_, _>>();
    let projected_keys = projected_map.keys().cloned().collect::<std::collections::BTreeSet<_>>();
    let actual_keys = actual_map.keys().cloned().collect::<std::collections::BTreeSet<_>>();

    let projection_only_orders = projected_keys
        .difference(&actual_keys)
        .filter_map(|key| projected_map.get(key).cloned())
        .collect::<Vec<_>>();
    let exchange_only_orders = actual_keys
        .difference(&projected_keys)
        .filter_map(|key| actual_map.get(key).cloned())
        .collect::<Vec<_>>();
    let matched_count = projected_keys.intersection(&actual_keys).count();

    (projection_only_orders, exchange_only_orders, matched_count)
}

fn compare_key(order: &ShadowOrderView) -> OrderCompareKey {
    OrderCompareKey {
        side: order.side.clone(),
        price: order.price.normalize().to_string(),
        qty: order.qty.normalize().to_string(),
        reduce_only: order.reduce_only,
        post_only: order.post_only,
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::{
        execution::{DryRunExecutionAdapter, ExecutionEngine, ExecutionPlan, ExistingOrder}, reconcile::ReconcileOutcome,
        service::ServiceCycleOutput, AppConfig, GridMode, OrderIntent, OrderSide, OrderType, PlannerOutput, Position,
        RuntimeHealth, RuntimeState,
    };

    fn cfg() -> AppConfig {
        AppConfig {
            symbol: "ETH_USDC_PERP".into(),
            levels: 5,
            spacing_bps: dec!(35),
            order_size: dec!(0.003),
            max_position_abs: dec!(0.02),
            grid_mode: GridMode::ShortOnly,
            grid_active_levels: 5,
            grid_short_bias_sell_ratio: dec!(3),
            grid_min_price: None,
            grid_max_price: None,
            leverage: dec!(10),
            quote_asset: "USDC".into(),
            kill_switch: false,
        }
    }

    fn existing(price: Decimal) -> ExistingOrder {
        let intent = OrderIntent {
            client_order_id: format!("sell-{price}"),
            symbol: "ETH_USDC_PERP".into(),
            side: OrderSide::Sell,
            order_type: OrderType::Limit,
            price,
            qty: dec!(0.003),
            reduce_only: false,
            post_only: true,
        };
        ExistingOrder {
            order_id: format!("id-{price}"),
            client_order_id: intent.client_order_id.clone(),
            intent,
        }
    }

    #[test]
    fn report_serializes_cycle_summary() {
        let engine = ExecutionEngine::new(cfg());
        let mut plan = ExecutionPlan::default();
        plan.keep = vec![existing(dec!(2268.32))];
        plan.place = vec![OrderIntent {
            client_order_id: "sell-2276.24".into(),
            symbol: "ETH_USDC_PERP".into(),
            side: OrderSide::Sell,
            order_type: OrderType::Limit,
            price: dec!(2276.24),
            qty: dec!(0.003),
            reduce_only: false,
            post_only: true,
        }];
        let executed = engine.execute(&DryRunExecutionAdapter::new(), &plan).unwrap();
        let cycle = ServiceCycleOutput {
            planner: PlannerOutput {
                active_levels: vec![],
                desired_orders: vec![],
                rejected_levels: vec![],
                projected_position_after_orders: dec!(-0.006),
            },
            execution: plan,
            executed: Some(executed),
            reconciliation: ReconcileOutcome::default(),
            events: vec![],
            state: RuntimeState {
                working_orders: vec![existing(dec!(2268.32))],
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
        assert!(json.contains("execution_summary"));
        assert!(json.contains("projection_summary"));
        assert_eq!(report.projection_summary.actual_open_order_count, 1);
        assert_eq!(report.projection_summary.projected_final_order_count, 2);
        assert_eq!(report.projection_summary.only_in_projection_count, 1);
        assert!(json.contains("dry_run"));
    }
}
