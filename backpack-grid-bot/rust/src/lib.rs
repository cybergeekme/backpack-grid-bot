pub mod adapter;
pub mod config;
pub mod domain;
pub mod planner;
pub mod precision;
pub mod risk;

pub use config::{AppConfig, GridMode};
pub use domain::{GridLevel, OrderIntent, OrderSide, OrderType, PlannerOutput, Position, RiskDecision, RiskReason};
pub use planner::GridPlanner;
pub use precision::{normalize_order_qty, normalize_position_size, order_precision_decimals, price_precision_decimals};
pub use risk::RiskEngine;
