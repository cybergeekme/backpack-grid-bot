use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderType {
    Limit,
    Market,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridMode {
    Neutral,
    ShortBias,
    ShortOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub size: Decimal,
    pub entry_price: Decimal,
    pub unrealized_pnl: Decimal,
}

impl Position {
    pub fn flat(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            size: Decimal::ZERO,
            entry_price: Decimal::ZERO,
            unrealized_pnl: Decimal::ZERO,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridLevel {
    pub index: usize,
    pub side: OrderSide,
    pub price: Decimal,
    pub qty: Decimal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderIntent {
    pub client_order_id: String,
    pub symbol: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub price: Decimal,
    pub qty: Decimal,
    pub reduce_only: bool,
    pub post_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskReason {
    KillSwitchEnabled,
    QtyMustBePositive,
    ShortOnlyLongFlipBlocked,
    MaxPositionBreached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskDecision {
    pub ok: bool,
    pub reason: Option<RiskReason>,
}

impl RiskDecision {
    pub fn allow() -> Self {
        Self { ok: true, reason: None }
    }

    pub fn reject(reason: RiskReason) -> Self {
        Self {
            ok: false,
            reason: Some(reason),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannerOutput {
    pub active_levels: Vec<GridLevel>,
    pub desired_orders: Vec<OrderIntent>,
    pub rejected_levels: Vec<(GridLevel, RiskReason)>,
    pub projected_position_after_orders: Decimal,
}
