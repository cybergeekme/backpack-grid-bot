use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{health::ServiceState, GridLevel, OrderIntent, SyntheticFill};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEventKind {
    ServiceStateChanged,
    GridPaused,
    OrderPlanned,
    OrderCancelled,
    OrderRetained,
    PlannerRejectedLevel,
    SyntheticFillInferred,
    ReconciliationMismatch,
    ProjectionDriftDetected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeEvent {
    pub kind: RuntimeEventKind,
    pub message: String,
    pub order: Option<OrderIntent>,
    pub level: Option<GridLevel>,
    pub fill: Option<SyntheticFill>,
    pub service_state: Option<ServiceState>,
    pub matched_orders: Option<usize>,
    pub projection_only_orders: Option<usize>,
    pub exchange_only_orders: Option<usize>,
    pub mark_price: Option<Decimal>,
}

impl RuntimeEvent {
    pub fn service_state_changed(service_state: ServiceState, message: impl Into<String>) -> Self {
        Self {
            kind: RuntimeEventKind::ServiceStateChanged,
            message: message.into(),
            order: None,
            level: None,
            fill: None,
            service_state: Some(service_state),
            matched_orders: None,
            projection_only_orders: None,
            exchange_only_orders: None,
            mark_price: None,
        }
    }

    pub fn paused(service_state: ServiceState, message: impl Into<String>, mark_price: Option<Decimal>) -> Self {
        Self {
            kind: RuntimeEventKind::GridPaused,
            message: message.into(),
            order: None,
            level: None,
            fill: None,
            service_state: Some(service_state),
            matched_orders: None,
            projection_only_orders: None,
            exchange_only_orders: None,
            mark_price,
        }
    }

    pub fn order_planned(kind: RuntimeEventKind, order: OrderIntent, mark_price: Option<Decimal>, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            order: Some(order),
            level: None,
            fill: None,
            service_state: None,
            matched_orders: None,
            projection_only_orders: None,
            exchange_only_orders: None,
            mark_price,
        }
    }

    pub fn planner_rejected(level: GridLevel, message: impl Into<String>, mark_price: Option<Decimal>) -> Self {
        Self {
            kind: RuntimeEventKind::PlannerRejectedLevel,
            message: message.into(),
            order: None,
            level: Some(level),
            fill: None,
            service_state: None,
            matched_orders: None,
            projection_only_orders: None,
            exchange_only_orders: None,
            mark_price,
        }
    }

    pub fn synthetic_fill(fill: SyntheticFill, mark_price: Option<Decimal>) -> Self {
        Self {
            kind: RuntimeEventKind::SyntheticFillInferred,
            message: "synthetic fill inferred during reconciliation".into(),
            order: None,
            level: None,
            fill: Some(fill),
            service_state: None,
            matched_orders: None,
            projection_only_orders: None,
            exchange_only_orders: None,
            mark_price,
        }
    }

    pub fn mismatch(matched_orders: usize, mark_price: Option<Decimal>) -> Self {
        Self {
            kind: RuntimeEventKind::ReconciliationMismatch,
            message: "desired orders and exchange state diverged".into(),
            order: None,
            level: None,
            fill: None,
            service_state: None,
            matched_orders: Some(matched_orders),
            projection_only_orders: None,
            exchange_only_orders: None,
            mark_price,
        }
    }

    pub fn projection_drift(
        projection_only_orders: usize,
        exchange_only_orders: usize,
        mark_price: Option<Decimal>,
    ) -> Self {
        Self {
            kind: RuntimeEventKind::ProjectionDriftDetected,
            message: format!(
                "projected final orders drift from exchange state (projection_only={}, exchange_only={})",
                projection_only_orders, exchange_only_orders
            ),
            order: None,
            level: None,
            fill: None,
            service_state: None,
            matched_orders: None,
            projection_only_orders: Some(projection_only_orders),
            exchange_only_orders: Some(exchange_only_orders),
            mark_price,
        }
    }
}
