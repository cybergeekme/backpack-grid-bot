use rust_decimal::Decimal;
use serde_json::Value;
use thiserror::Error;

use crate::domain::{OrderIntent, OrderSide, OrderType, Position};

use super::dto::{BackpackBalanceRow, BackpackCollateralSummary, BackpackOrderRow, BackpackPositionRow};

#[derive(Debug, Error)]
pub enum BackpackAdapterError {
    #[error("invalid json payload shape for {0}")]
    InvalidPayload(&'static str),
}

pub struct BackpackNormalize;

impl BackpackNormalize {
    pub fn map_side(raw: &str) -> OrderSide {
        if raw.eq_ignore_ascii_case("ask") || raw.eq_ignore_ascii_case("sell") {
            OrderSide::Sell
        } else {
            OrderSide::Buy
        }
    }

    pub fn map_order_type(raw: Option<&str>) -> OrderType {
        match raw.unwrap_or("limit").to_ascii_lowercase().as_str() {
            "market" => OrderType::Market,
            _ => OrderType::Limit,
        }
    }

    pub fn is_open_status(raw: Option<&str>) -> bool {
        matches!(
            raw.unwrap_or("open"),
            "Accepted" | "New" | "Open" | "accepted" | "new" | "open"
        )
    }

    pub fn normalize_position_payload(payload: &Value, symbol: &str) -> Option<BackpackPositionRow> {
        match payload {
            Value::Array(rows) => {
                let parsed = rows
                    .iter()
                    .filter_map(|entry| serde_json::from_value::<BackpackPositionRow>(rewrite_position_keys(entry.clone())).ok())
                    .collect::<Vec<_>>();
                parsed
                    .iter()
                    .find(|row| row.symbol.as_deref() == Some(symbol))
                    .cloned()
                    .or_else(|| parsed.into_iter().next())
            }
            Value::Object(_) => serde_json::from_value::<BackpackPositionRow>(rewrite_position_keys(payload.clone())).ok(),
            _ => None,
        }
    }

    pub fn normalize_position(payload: &Value, symbol: &str) -> Position {
        let row = match Self::normalize_position_payload(payload, symbol) {
            Some(row) => row,
            None => return Position::flat(symbol.to_string()),
        };

        Position {
            symbol: symbol.to_string(),
            size: row.net_quantity.or(row.quantity).or(row.position_qty).unwrap_or(Decimal::ZERO),
            entry_price: row.entry_price.or(row.average_entry_price).unwrap_or(Decimal::ZERO),
            unrealized_pnl: row.unrealized_pnl.or(row.pnl_unrealized).or(row.pnl).unwrap_or(Decimal::ZERO),
        }
    }

    pub fn normalize_collateral_summary(payload: &Value) -> Option<BackpackCollateralSummary> {
        match payload {
            Value::Object(_) => serde_json::from_value::<BackpackCollateralSummary>(rewrite_collateral_keys(payload.clone())).ok(),
            _ => None,
        }
    }

    pub fn normalize_balances(payload: &Value) -> Vec<BackpackBalanceRow> {
        match payload {
            Value::Array(items) => items
                .iter()
                .filter_map(|entry| serde_json::from_value::<BackpackBalanceRow>(rewrite_balance_keys(entry.clone())).ok())
                .collect(),
            Value::Object(map) => {
                if let Some(nested) = map.get("balances").or_else(|| map.get("capital")).or_else(|| map.get("items")) {
                    return Self::normalize_balances(nested);
                }
                map.iter()
                    .filter_map(|(asset, value)| {
                        let mut row = rewrite_balance_keys(value.clone());
                        if let Value::Object(ref mut obj) = row {
                            obj.entry("asset".to_string()).or_insert(Value::String(asset.clone()));
                        }
                        serde_json::from_value::<BackpackBalanceRow>(row).ok()
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    pub fn normalize_order(row: &BackpackOrderRow, fallback: Option<&OrderIntent>) -> Option<OrderIntent> {
        let symbol = row.symbol.clone().or_else(|| fallback.map(|f| f.symbol.clone()))?;
        let side = row
            .side
            .as_deref()
            .map(Self::map_side)
            .or_else(|| fallback.map(|f| f.side))
            .unwrap_or(OrderSide::Buy);
        let order_type = row
            .order_type
            .as_deref()
            .map(|raw| Self::map_order_type(Some(raw)))
            .or_else(|| fallback.map(|f| f.order_type))
            .unwrap_or(OrderType::Limit);
        let price = row.price.or_else(|| fallback.map(|f| f.price)).unwrap_or(Decimal::ZERO);
        let qty = row.quantity.or(row.qty).or_else(|| fallback.map(|f| f.qty)).unwrap_or(Decimal::ZERO);

        Some(OrderIntent {
            client_order_id: row
                .client_order_id
                .clone()
                .or_else(|| row.client_id.clone())
                .or_else(|| fallback.map(|f| f.client_order_id.clone()))
                .unwrap_or_default(),
            symbol,
            side,
            order_type,
            price,
            qty,
            reduce_only: row.reduce_only.or_else(|| fallback.map(|f| f.reduce_only)).unwrap_or(false),
            post_only: row.post_only.or_else(|| fallback.map(|f| f.post_only)).unwrap_or(false),
        })
    }

    pub fn open_orders_from_payload(payload: &Value) -> Vec<BackpackOrderRow> {
        match payload {
            Value::Array(items) => items
                .iter()
                .filter_map(|entry| serde_json::from_value::<BackpackOrderRow>(rewrite_order_keys(entry.clone())).ok())
                .filter(|row| Self::is_open_status(row.status.as_deref()))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn collateral_available(summary: &BackpackCollateralSummary, asset: &str) -> Option<(Decimal, Decimal)> {
        if let (Some(total), Some(available)) = (summary.net_equity, summary.net_equity_available) {
            if total > Decimal::ZERO || available > Decimal::ZERO {
                return Some((total, available));
            }
        }

        summary
            .collateral
            .iter()
            .find(|row| row.symbol.as_deref() == Some(asset))
            .map(|row| {
                let total = row.total_quantity.or(row.balance_notional).unwrap_or(Decimal::ZERO);
                let available = row.available_quantity.or(row.balance_notional).unwrap_or(total);
                (total, available)
            })
    }
}

fn rewrite_balance_keys(value: Value) -> Value {
    rename_keys(value, &[("availableBalance", "available_balance")])
}

fn rewrite_position_keys(value: Value) -> Value {
    rename_keys(
        value,
        &[
            ("netQuantity", "net_quantity"),
            ("positionQty", "position_qty"),
            ("entryPrice", "entry_price"),
            ("averageEntryPrice", "average_entry_price"),
            ("unrealizedPnl", "unrealized_pnl"),
            ("pnlUnrealized", "pnl_unrealized"),
        ],
    )
}

fn rewrite_collateral_keys(value: Value) -> Value {
    let value = rename_keys(value, &[("netEquity", "net_equity"), ("netEquityAvailable", "net_equity_available")]);
    match value {
        Value::Object(mut map) => {
            if let Some(Value::Array(rows)) = map.remove("collateral") {
                let rewritten = rows
                    .into_iter()
                    .map(|row| rename_keys(row, &[("totalQuantity", "total_quantity"), ("availableQuantity", "available_quantity"), ("balanceNotional", "balance_notional")]))
                    .collect::<Vec<_>>();
                map.insert("collateral".to_string(), Value::Array(rewritten));
            }
            Value::Object(map)
        }
        other => other,
    }
}

fn rewrite_order_keys(value: Value) -> Value {
    rename_keys(
        value,
        &[
            ("orderId", "order_id"),
            ("clientOrderId", "client_order_id"),
            ("clientId", "client_id"),
            ("orderType", "order_type"),
            ("executedQuantity", "executed_quantity"),
            ("filledQuantity", "filled_quantity"),
            ("filledQty", "filled_qty"),
            ("reduceOnly", "reduce_only"),
            ("postOnly", "post_only"),
            ("createdAt", "created_at"),
        ],
    )
}

fn rename_keys(value: Value, mappings: &[(&str, &str)]) -> Value {
    match value {
        Value::Object(mut map) => {
            for (from, to) in mappings {
                if let Some(v) = map.remove(*from) {
                    map.insert((*to).to_string(), v);
                }
            }
            Value::Object(map)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use serde_json::json;

    #[test]
    fn normalizes_array_position_payload() {
        let payload = json!([
            {"symbol":"BTC_USDC_PERP","netQuantity":"0.1","entryPrice":"50000"},
            {"symbol":"ETH_USDC_PERP","netQuantity":"-0.003","averageEntryPrice":"2255.45","pnl":"1.2"}
        ]);
        let position = BackpackNormalize::normalize_position(&payload, "ETH_USDC_PERP");
        assert_eq!(position.size, dec!(-0.003));
        assert_eq!(position.entry_price, dec!(2255.45));
        assert_eq!(position.unrealized_pnl, dec!(1.2));
    }

    #[test]
    fn normalizes_collateral_summary() {
        let payload = json!({
            "netEquity": "149.394572",
            "netEquityAvailable": "147.359132",
            "collateral": [
                {"symbol":"USDC","totalQuantity":"149.394572","availableQuantity":"147.359132"}
            ]
        });
        let summary = BackpackNormalize::normalize_collateral_summary(&payload).unwrap();
        let (total, available) = BackpackNormalize::collateral_available(&summary, "USDC").unwrap();
        assert_eq!(total, dec!(149.394572));
        assert_eq!(available, dec!(147.359132));
    }

    #[test]
    fn filters_open_orders() {
        let payload = json!([
            {"orderId":"1","status":"Open","symbol":"ETH_USDC_PERP","side":"Ask","orderType":"Limit","price":"2260.4","quantity":"0.003","reduceOnly":false,"postOnly":true},
            {"orderId":"2","status":"Filled","symbol":"ETH_USDC_PERP","side":"Bid","orderType":"Limit","price":"2252.48","quantity":"0.003","reduceOnly":true,"postOnly":true}
        ]);
        let orders = BackpackNormalize::open_orders_from_payload(&payload);
        assert_eq!(orders.len(), 1);
        let intent = BackpackNormalize::normalize_order(&orders[0], None).unwrap();
        assert_eq!(intent.side, OrderSide::Sell);
        assert_eq!(intent.qty, dec!(0.003));
    }
}
