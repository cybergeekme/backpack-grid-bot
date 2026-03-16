use rust_decimal::Decimal;

use crate::{
    adapter::ReadOnlyAccountSnapshot,
    events::{RuntimeEvent, RuntimeEventKind},
    execution::{DryRunExecutionAdapter, ExecutedPlan, ExecutionEngine, ExecutionPlan, ExistingOrder},
    health::{HealthIssue, PauseReason, RuntimeHealth, ServiceState},
    planner::GridPlanner,
    reconcile::{ReconcileEngine, ReconcileOutcome, ReconcileSnapshot},
    state::{InMemoryStateStore, RuntimeState},
    AppConfig, PlannerOutput, Position,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceCycleOutput {
    pub planner: PlannerOutput,
    pub execution: ExecutionPlan,
    pub executed: Option<ExecutedPlan>,
    pub reconciliation: ReconcileOutcome,
    pub events: Vec<RuntimeEvent>,
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

        if let Some(pause_reason) = self.guard_pause_reason(mid_price) {
            let service_state = ServiceState::Paused;
            let mut events = Vec::new();
            if previous.health.service_state != service_state {
                events.push(RuntimeEvent::service_state_changed(
                    service_state,
                    format!("service state changed from {:?} to {:?}", previous.health.service_state, service_state),
                ));
            }
            events.push(RuntimeEvent::paused(
                service_state,
                pause_reason_message(pause_reason, mid_price),
                Some(mid_price),
            ));
            for event in events.iter().cloned() {
                self.state.push_event(event, 200);
            }

            let mut health = RuntimeHealth::default();
            health.pause(pause_reason, Some(mid_price));
            self.state.set_health(health);

            return ServiceCycleOutput {
                planner: PlannerOutput {
                    active_levels: vec![],
                    desired_orders: vec![],
                    rejected_levels: vec![],
                    projected_position_after_orders: current_position.size,
                },
                execution: ExecutionPlan::default(),
                executed: None,
                reconciliation: ReconcileOutcome::default(),
                events,
                state: self.state.get(),
            };
        }

        let planner_output = self.planner.plan_orders(mid_price, Some(&current_position));
        let execution_plan = self.execution.plan_from_planner_output(&planner_output, &current_orders);
        let executed = self.execution.execute(&DryRunExecutionAdapter::new(), &execution_plan).ok();
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

        let events = build_runtime_events(
            &planner_output,
            &execution_plan,
            executed.as_ref(),
            &reconciliation,
            mid_price,
            previous.health.service_state,
        );

        if let Some(fill) = reconciliation.synthetic_fill.clone() {
            self.state.push_fill(fill, 100);
        }
        for event in events.iter().cloned() {
            self.state.push_event(event, 200);
        }

        let health = build_runtime_health(mid_price, &current_position, &execution_plan, &reconciliation);
        self.state.set_health(health);

        ServiceCycleOutput {
            planner: planner_output,
            execution: execution_plan,
            executed,
            reconciliation,
            events,
            state: self.state.get(),
        }
    }

    pub fn plan_cycle_from_snapshot(&mut self, snapshot: ReadOnlyAccountSnapshot) -> ServiceCycleOutput {
        self.plan_cycle(snapshot.mark_price, snapshot.position, snapshot.open_orders)
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    fn guard_pause_reason(&self, mid_price: Decimal) -> Option<PauseReason> {
        if self.config.kill_switch {
            return Some(PauseReason::KillSwitch);
        }
        if self.config.grid_min_price.is_some_and(|min| mid_price < min)
            || self.config.grid_max_price.is_some_and(|max| mid_price > max)
        {
            return Some(PauseReason::PriceOutOfRange);
        }
        None
    }
}

fn build_runtime_events(
    planner: &PlannerOutput,
    execution: &ExecutionPlan,
    executed: Option<&ExecutedPlan>,
    reconciliation: &ReconcileOutcome,
    mid_price: Decimal,
    previous_service_state: ServiceState,
) -> Vec<RuntimeEvent> {
    let mut events = Vec::new();

    let next_service_state = derive_service_state(execution, reconciliation);
    if next_service_state != previous_service_state {
        events.push(RuntimeEvent::service_state_changed(
            next_service_state,
            format!("service state changed from {:?} to {:?}", previous_service_state, next_service_state),
        ));
    }

    for order in &execution.place {
        events.push(RuntimeEvent::order_planned(
            RuntimeEventKind::OrderPlanned,
            order.clone(),
            Some(mid_price),
            format!("planned order {}", order.client_order_id),
        ));
    }
    for order in &execution.cancel {
        events.push(RuntimeEvent::order_planned(
            RuntimeEventKind::OrderCancelled,
            order.intent.clone(),
            Some(mid_price),
            format!("cancel order {}", order.client_order_id),
        ));
    }
    for order in &execution.keep {
        events.push(RuntimeEvent::order_planned(
            RuntimeEventKind::OrderRetained,
            order.intent.clone(),
            Some(mid_price),
            format!("keep order {}", order.client_order_id),
        ));
    }
    if let Some(executed) = executed {
        for order in &executed.placed {
            events.push(RuntimeEvent::order_planned(
                RuntimeEventKind::OrderPlanned,
                order.intent.clone(),
                Some(mid_price),
                format!("dry-run placed order {}", order.client_order_id),
            ));
        }
    }
    for (level, reason) in &planner.rejected_levels {
        events.push(RuntimeEvent::planner_rejected(
            level.clone(),
            format!("planner rejected level {:?}", reason),
            Some(mid_price),
        ));
    }
    if let Some(fill) = reconciliation.synthetic_fill.clone() {
        events.push(RuntimeEvent::synthetic_fill(fill, Some(mid_price)));
    }
    if !reconciliation.diff.missing_on_exchange.is_empty() || !reconciliation.diff.unexpected_on_exchange.is_empty() {
        events.push(RuntimeEvent::mismatch(reconciliation.diff.matched, Some(mid_price)));
    }

    events
}

fn build_runtime_health(
    mid_price: Decimal,
    current_position: &Position,
    execution: &ExecutionPlan,
    reconciliation: &ReconcileOutcome,
) -> RuntimeHealth {
    let mut issues = Vec::new();
    if reconciliation.synthetic_fill.is_some() {
        issues.push(HealthIssue::SyntheticFillObserved);
    }
    if !reconciliation.diff.missing_on_exchange.is_empty() || !reconciliation.diff.unexpected_on_exchange.is_empty() {
        issues.push(HealthIssue::OpenOrderMismatch);
    }
    if current_position.symbol.is_empty() {
        issues.push(HealthIssue::PositionMissing);
    }
    if mid_price <= Decimal::ZERO {
        issues.push(HealthIssue::MarkPriceMissing);
    }

    let service_state = derive_service_state(execution, reconciliation);
    let mut health = RuntimeHealth::default();
    health.mark_success(service_state, issues, Some(mid_price));
    health
}

fn derive_service_state(execution: &ExecutionPlan, reconciliation: &ReconcileOutcome) -> ServiceState {
    if !reconciliation.diff.missing_on_exchange.is_empty() || !reconciliation.diff.unexpected_on_exchange.is_empty() {
        ServiceState::Degraded
    } else if execution.place.is_empty() && execution.cancel.is_empty() && execution.keep.is_empty() {
        ServiceState::Starting
    } else {
        ServiceState::Active
    }
}

fn pause_reason_message(reason: PauseReason, mid_price: Decimal) -> String {
    match reason {
        PauseReason::KillSwitch => "grid paused by kill switch".to_string(),
        PauseReason::PriceOutOfRange => format!("grid paused: mark price {mid_price} out of configured range"),
        PauseReason::Manual => "grid paused manually".to_string(),
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
    use crate::{domain::OrderType, GridMode, OrderIntent, OrderSide, RuntimeEventKind, ServiceState};

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
    fn service_cycle_plans_orders_records_events_and_health() {
        let mut service = GridBotService::new(cfg());
        service.restore(RuntimeState {
            working_orders: vec![existing("1", OrderSide::Sell, dec!(2268.32), dec!(0.003), false)],
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

        assert_eq!(out.state.health.service_state, ServiceState::Degraded);
        assert!(!out.execution.place.is_empty());
        assert!(out.executed.is_some());
        assert!(out.events.iter().any(|event| event.kind == RuntimeEventKind::OrderPlanned));
        assert!(out.events.iter().any(|event| event.kind == RuntimeEventKind::ReconciliationMismatch));
    }

    #[test]
    fn service_cycle_pauses_when_kill_switch_enabled() {
        let mut config = cfg();
        config.kill_switch = true;
        let mut service = GridBotService::new(config);

        let out = service.plan_cycle(
            dec!(2260.4),
            Position {
                symbol: "ETH_USDC_PERP".into(),
                size: dec!(0),
                entry_price: dec!(0),
                unrealized_pnl: dec!(0),
            },
            vec![],
        );

        assert_eq!(out.state.health.service_state, ServiceState::Paused);
        assert_eq!(out.state.health.pause_reason, Some(PauseReason::KillSwitch));
        assert!(out.execution.place.is_empty());
        assert!(out.executed.is_none());
        assert!(out.events.iter().any(|event| event.kind == RuntimeEventKind::GridPaused));
    }

    #[test]
    fn service_cycle_pauses_when_price_out_of_range() {
        let mut config = cfg();
        config.grid_max_price = Some(dec!(2250));
        let mut service = GridBotService::new(config);

        let out = service.plan_cycle(
            dec!(2260.4),
            Position {
                symbol: "ETH_USDC_PERP".into(),
                size: dec!(0),
                entry_price: dec!(0),
                unrealized_pnl: dec!(0),
            },
            vec![],
        );

        assert_eq!(out.state.health.service_state, ServiceState::Paused);
        assert_eq!(out.state.health.pause_reason, Some(PauseReason::PriceOutOfRange));
        assert!(out.execution.place.is_empty());
        assert!(out.executed.is_none());
        assert!(out.events.iter().any(|event| event.kind == RuntimeEventKind::GridPaused));
    }
}
