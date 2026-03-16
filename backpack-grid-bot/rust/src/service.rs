use rust_decimal::Decimal;

use crate::{
    adapter::ReadOnlyAccountSnapshot,
    execution::{ExecutionEngine, ExecutionPlan, ExistingOrder},
    planner::GridPlanner,
    reconcile::{ReconcileEngine, ReconcileOutcome, ReconcileSnapshot},
    state::{InMemoryStateStore, RuntimeState},
    AppConfig, PlannerOutput, Position,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceCycleOutput {
    pub planner: PlannerOutput,
    pub execution: ExecutionPlan,
    pub reconciliation: ReconcileOutcome,
    pub state: RuntimeState,
}

#[derive(Debug, Clone)]
pub struct GridBotService {
    config: AppConfig,
    planner: GridPlanner,
    execution: ExecutionEngine,
    reconcile: ReconcileEngine,
    state: InMemoryStateStore,
}

impl GridBotService {
    pub fn new(config: AppConfig) -> Self {
        let planner = GridPlanner::new(config.clone());
        let execution = ExecutionEngine::new(config.clone());
        let reconcile = ReconcileEngine::new(config.symbol.clone(), qty_epsilon(&config.order_size));
        Self {
            config,
            planner,
            execution,
            reconcile,
            state: InMemoryStateStore::new(),
        }
    }

    pub fn restore(&mut self, state: RuntimeState) {
        self.state.restore(state);
    }

    pub fn snapshot(&self) -> RuntimeState {
        self.state.get()
    }

    pub fn plan_cycle(
        &mut self,
        mid_price: Decimal,
        current_position: Position,
        current_orders: Vec<ExistingOrder>,
    ) -> ServiceCycleOutput {
        let previous = self.state.get();
        self.state.set_last_mid_price(mid_price);
        self.state.set_position(current_position.clone());
        self.state.set_working_orders(current_orders.clone());

        let planner_output = self.planner.plan_orders(mid_price, Some(&current_position));
        let execution_plan = self.execution.plan_from_planner_output(&planner_output, &current_orders);
        let reconciliation = self.reconcile.reconcile(
            &ReconcileSnapshot {
                position: previous.position.clone(),
                orders: previous.working_orders.clone(),
            },
            &ReconcileSnapshot {
                position: Some(current_position.clone()),
                orders: current_orders,
            },
            &planner_output.desired_orders,
        );

        if let Some(fill) = reconciliation.synthetic_fill.clone() {
            self.state.push_fill(fill, 100);
        }

        ServiceCycleOutput {
            planner: planner_output,
            execution: execution_plan,
            reconciliation,
            state: self.state.get(),
        }
    }

    pub fn plan_cycle_from_snapshot(&mut self, snapshot: ReadOnlyAccountSnapshot) -> ServiceCycleOutput {
        self.plan_cycle(snapshot.mark_price, snapshot.position, snapshot.open_orders)
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }
}

fn qty_epsilon(order_size: &Decimal) -> Decimal {
    let decimals = order_size.normalize().to_string().split('.').nth(1).map(|s| s.len()).unwrap_or(0) as u32;
    Decimal::new(1, decimals + 3)
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::{domain::OrderType, GridMode, OrderIntent, OrderSide};

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

    fn existing(order_id: &str, side: OrderSide, price: Decimal, qty: Decimal, reduce_only: bool) -> ExistingOrder {
        let intent = OrderIntent {
            client_order_id: format!("{:?}-{price}", side),
            symbol: "ETH_USDC_PERP".into(),
            side,
            order_type: OrderType::Limit,
            price,
            qty,
            reduce_only,
            post_only: true,
        };
        ExistingOrder {
            order_id: order_id.into(),
            client_order_id: intent.client_order_id.clone(),
            intent,
        }
    }

    #[test]
    fn service_cycle_plans_orders_and_records_reconcile_fill() {
        let mut service = GridBotService::new(cfg());
        service.restore(RuntimeState {
            working_orders: vec![existing("1", OrderSide::Sell, dec!(2268.32), dec!(0.003), false)],
            recent_fills: vec![],
            position: Some(Position {
                symbol: "ETH_USDC_PERP".into(),
                size: dec!(-0.003),
                entry_price: dec!(2255.45),
                unrealized_pnl: dec!(0),
            }),
            last_mid_price: Some(dec!(2260.4)),
        });

        let out = service.plan_cycle(
            dec!(2260.4),
            Position {
                symbol: "ETH_USDC_PERP".into(),
                size: dec!(-0.006),
                entry_price: dec!(2255.45),
                unrealized_pnl: dec!(0),
            },
            vec![],
        );

        assert!(!out.planner.desired_orders.is_empty());
        assert!(out.reconciliation.synthetic_fill.is_some());
        assert_eq!(out.state.recent_fills.len(), 1);
        assert_eq!(out.state.recent_fills[0].side, OrderSide::Sell);
    }
}
