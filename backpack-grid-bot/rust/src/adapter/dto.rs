use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BackpackBalanceRow {
    pub asset: Option<String>,
    pub total: Option<Decimal>,
    pub balance: Option<Decimal>,
    pub available: Option<Decimal>,
    pub available_balance: Option<Decimal>,
    pub free: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BackpackCollateralRow {
    pub symbol: Option<String>,
    pub total_quantity: Option<Decimal>,
    pub available_quantity: Option<Decimal>,
    pub balance_notional: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BackpackCollateralSummary {
    pub net_equity: Option<Decimal>,
    pub net_equity_available: Option<Decimal>,
    #[serde(default)]
    pub collateral: Vec<BackpackCollateralRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BackpackPositionRow {
    pub symbol: Option<String>,
    pub net_quantity: Option<Decimal>,
    pub quantity: Option<Decimal>,
    pub position_qty: Option<Decimal>,
    pub entry_price: Option<Decimal>,
    pub average_entry_price: Option<Decimal>,
    pub unrealized_pnl: Option<Decimal>,
    pub pnl_unrealized: Option<Decimal>,
    pub pnl: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BackpackOrderRow {
    pub order_id: Option<String>,
    pub id: Option<String>,
    pub client_order_id: Option<String>,
    pub client_id: Option<String>,
    pub symbol: Option<String>,
    pub side: Option<String>,
    pub order_type: Option<String>,
    pub status: Option<String>,
    pub price: Option<Decimal>,
    pub quantity: Option<Decimal>,
    pub qty: Option<Decimal>,
    pub executed_quantity: Option<Decimal>,
    pub filled_quantity: Option<Decimal>,
    pub filled_qty: Option<Decimal>,
    pub reduce_only: Option<bool>,
    pub post_only: Option<bool>,
    pub timestamp: Option<i64>,
    pub created_at: Option<i64>,
}
