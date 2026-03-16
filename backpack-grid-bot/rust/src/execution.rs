use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    domain::{OrderIntent, OrderSide, PlannerOutput},
    precision::normalize_order_qty,
    AppConfig,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OrderKey {
    pub side: OrderSide,
    pub price_key: String,
    pub qty_key: String,
    pub reduce_only: bool,
    pub post_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExistingOrder {
    pub order_id: String,
    pub client_order_id: String,
    pub intent: OrderIntent,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReconciliationDiff {
    pub missing_on_exchange: Vec<OrderIntent>,
    pub unexpected_on_exchange: Vec<ExistingOrder>,
    pub matched: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExecutionPlan {
    pub cancel: Vec<ExistingOrder>,
    pub place: Vec<OrderIntent>,
    pub keep: Vec<ExistingOrder>,
    pub diff: ReconciliationDiff,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SyncGridResult {
    pub final_orders: Vec<ExistingOrder>,
    pub placed: Vec<OrderIntent>,
    pub cancelled: Vec<ExistingOrder>,
    pub diff: ReconciliationDiff,
}

#[derive(Debug, Clone)]
pub struct ExecutionEngine {
    config: AppConfig,
}

impl ExecutionEngine {
    pub fn new(config: AppConfig) -> Self {
        Self { config }
    }

    pub fn build_plan(&self, desired: &[OrderIntent], existing: &[ExistingOrder]) -> ExecutionPlan {
        let desired_index = self.index_desired(desired);
        let existing_index = self.index_existing(existing);

        let desired_keys = desired_index.keys().cloned().collect::<BTreeSet<_>>();
        let existing_keys = existing_index.keys().cloned().collect::<BTreeSet<_>>();

        let place = desired_keys
            .difference(&existing_keys)
            .filter_map(|key| desired_index.get(key).cloned())
            .collect::<Vec<_>>();

        let cancel = existing_keys
            .difference(&desired_keys)
            .filter_map(|key| existing_index.get(key).cloned())
            .collect::<Vec<_>>();

        let keep = existing_keys
            .intersection(&desired_keys)
            .filter_map(|key| existing_index.get(key).cloned())
            .collect::<Vec<_>>();

        let diff = ReconciliationDiff {
            missing_on_exchange: place.clone(),
            unexpected_on_exchange: cancel.clone(),
            matched: keep.len(),
        };

        ExecutionPlan {
            cancel,
            place,
            keep,
            diff,
        }
    }

    pub fn project_sync_result(&self, plan: &ExecutionPlan, final_orders: &[ExistingOrder]) -> SyncGridResult {
        SyncGridResult {
            final_orders: final_orders.to_vec(),
            placed: plan.place.clone(),
            cancelled: plan.cancel.clone(),
            diff: self.build_plan(
                &final_orders.iter().map(|order| order.intent.clone()).collect::<Vec<_>>(),
                final_orders,
            )
            .diff,
        }
    }

    pub fn plan_from_planner_output(&self, planner_output: &PlannerOutput, existing: &[ExistingOrder]) -> ExecutionPlan {
        self.build_plan(&planner_output.desired_orders, existing)
    }

    pub fn order_key(&self, intent: &OrderIntent) -> OrderKey {
        OrderKey {
            side: intent.side,
            price_key: decimal_key(intent.price),
            qty_key: decimal_key(normalize_order_qty(intent.qty, self.config.order_size)),
            reduce_only: intent.reduce_only,
            post_only: intent.post_only,
        }
    }

    fn index_desired(&self, desired: &[OrderIntent]) -> BTreeMap<OrderKey, OrderIntent> {
        desired
            .iter()
            .cloned()
            .map(|intent| (self.order_key(&intent), intent))
            .collect()
    }

    fn index_existing(&self, existing: &[ExistingOrder]) -> BTreeMap<OrderKey, ExistingOrder> {
        existing
            .iter()
            .cloned()
            .map(|order| (self.order_key(&order.intent), order))
            .collect()
    }
}

fn decimal_key(value: Decimal) -> String {
    value.normalize().to_string()
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::{GridMode, Position};

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

    fn intent(side: OrderSide, price: Decimal, qty: Decimal, reduce_only: bool) -> OrderIntent {
        OrderIntent {
            client_order_id: format!("{:?}-{}", side, price),
            symbol: "ETH_USDC_PERP".into(),
            side,
            order_type: crate::domain::OrderType::Limit,
            price,
            qty,
            reduce_only,
            post_only: true,
        }
    }

    fn existing(order_id: &str, intent: OrderIntent) -> ExistingOrder {
        ExistingOrder {
            order_id: order_id.into(),
            client_order_id: intent.client_order_id.clone(),
            intent,
        }
    }

    #[test]
    fn build_plan_detects_missing_and_unexpected_orders() {
        let engine = ExecutionEngine::new(cfg());
        let desired = vec![
            intent(OrderSide::Buy, dec!(2252.48), dec!(0.003), true),
            intent(OrderSide::Sell, dec!(2268.32), dec!(0.003), false),
        ];
        let existing_orders = vec![
            existing("1", intent(OrderSide::Buy, dec!(2252.48), dec!(0.0030000000000000005), true)),
            existing("2", intent(OrderSide::Sell, dec!(2276.24), dec!(0.003), false)),
        ];

        let plan = engine.build_plan(&desired, &existing_orders);
        assert_eq!(plan.keep.len(), 1);
        assert_eq!(plan.place.len(), 1);
        assert_eq!(plan.cancel.len(), 1);
        assert_eq!(plan.diff.matched, 1);
        assert_eq!(plan.place[0].price, dec!(2268.32));
        assert_eq!(plan.cancel[0].intent.price, dec!(2276.24));
    }

    #[test]
    fn planner_output_flows_into_execution_plan() {
        let cfg = cfg();
        let planner = crate::planner::GridPlanner::new(cfg.clone());
        let engine = ExecutionEngine::new(cfg);
        let position = Position {
            symbol: "ETH_USDC_PERP".into(),
            size: dec!(-0.006),
            entry_price: dec!(2262.38),
            unrealized_pnl: Decimal::ZERO,
        };
        let planner_output = planner.plan_orders(dec!(2260.4), Some(&position));
        let existing_orders = vec![];

        let plan = engine.plan_from_planner_output(&planner_output, &existing_orders);
        assert_eq!(plan.place.len(), planner_output.desired_orders.len());
        assert!(plan.cancel.is_empty());
    }
}
