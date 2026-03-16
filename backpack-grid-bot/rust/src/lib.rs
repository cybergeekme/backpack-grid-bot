pub mod adapter;
pub mod config;
pub mod domain;
pub mod execution;
pub mod planner;
pub mod precision;
pub mod reconcile;
pub mod risk;

pub use config::{AppConfig, GridMode};
pub use domain::{GridLevel, OrderIntent, OrderSide, OrderType, PlannerOutput, Position, RiskDecision, RiskReason};
pub use execution::{ExecutionEngine, ExecutionPlan, ExistingOrder, ReconciliationDiff, SyncGridResult};
pub use planner::GridPlanner;
pub use precision::{normalize_order_qty, normalize_position_size, order_precision_decimals, price_precision_decimals};
pub use reconcile::{FillMetrics, FillSource, ReconcileEngine, ReconcileOutcome, ReconcileSnapshot, SyntheticFill};
pub use risk::RiskEngine;
