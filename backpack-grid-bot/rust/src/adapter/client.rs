use std::time::Duration;

use anyhow::Result;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE};
use serde_json::Value;

use crate::{
    adapter::{BackpackCollateralSummary, BackpackNormalize, BackpackOrderRow},
    execution::ExistingOrder,
    Position,
};

use super::BackpackAdapterError;

#[derive(Debug, Clone)]
pub struct BackpackHttpClient {
    base_url: String,
    api_key: Option<String>,
    api_secret: Option<String>,
    client: Client,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadOnlyAccountSnapshot {
    pub symbol: String,
    pub mark_price: rust_decimal::Decimal,
    pub position: Position,
    pub open_orders: Vec<ExistingOrder>,
    pub collateral: Option<BackpackCollateralSummary>,
}

impl BackpackHttpClient {
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        api_secret: Option<String>,
    ) -> Result<Self> {
        let client = Client::builder().timeout(Duration::from_secs(15)).build()?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key,
            api_secret,
            client,
        })
    }

    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("BACKPACK_API_BASE_URL")
            .unwrap_or_else(|_| "https://api.backpack.exchange".to_string());
        let api_key = std::env::var("BACKPACK_API_KEY").ok();
        let api_secret = std::env::var("BACKPACK_API_SECRET").ok();
        Self::new(base_url, api_key, api_secret)
    }

    pub fn fetch_mark_price(&self, symbol: &str) -> Result<rust_decimal::Decimal> {
        let payload = self.get_json("/api/v1/ticker", &[("symbol", symbol)])?;
        extract_decimal_field(&payload, &["markPrice", "mark_price", "lastPrice", "last_price", "price"])
            .ok_or_else(|| BackpackAdapterError::InvalidPayload("ticker").into())
    }

    pub fn fetch_position(&self, symbol: &str) -> Result<Position> {
        let payload = self.get_json("/api/v1/position", &[("symbol", symbol)])?;
        Ok(BackpackNormalize::normalize_position(&payload, symbol))
    }

    pub fn fetch_open_orders(&self, symbol: &str) -> Result<Vec<ExistingOrder>> {
        let payload = self.get_json("/api/v1/orders", &[("symbol", symbol)])?;
        let rows = BackpackNormalize::open_orders_from_payload(&payload);
        Ok(rows
            .into_iter()
            .filter_map(|row| normalize_existing_order(&row))
            .collect())
    }

    pub fn fetch_collateral_summary(&self) -> Result<Option<BackpackCollateralSummary>> {
        let payload = self.get_json("/api/v1/capital/collateral", &[])?;
        Ok(BackpackNormalize::normalize_collateral_summary(&payload))
    }

    pub fn fetch_snapshot(&self, symbol: &str) -> Result<ReadOnlyAccountSnapshot> {
        Ok(ReadOnlyAccountSnapshot {
            symbol: symbol.to_string(),
            mark_price: self.fetch_mark_price(symbol)?,
            position: self.fetch_position(symbol)?,
            open_orders: self.fetch_open_orders(symbol)?,
            collateral: self.fetch_collateral_summary()?,
        })
    }

    fn get_json(&self, path: &str, params: &[(&str, &str)]) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let request = self.client.get(url).query(params);
        let response = self.with_auth_headers(request).send()?.error_for_status()?;
        Ok(response.json()?)
    }

    fn with_auth_headers(&self, request: RequestBuilder) -> RequestBuilder {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(api_key) = &self.api_key {
            if let Ok(value) = HeaderValue::from_str(api_key) {
                headers.insert("X-API-KEY", value);
            }
        }
        if let Some(api_secret) = &self.api_secret {
            if let Ok(value) = HeaderValue::from_str(api_secret) {
                headers.insert("X-API-SECRET", value);
            }
        }
        request.headers(headers)
    }
}

fn normalize_existing_order(row: &BackpackOrderRow) -> Option<ExistingOrder> {
    let intent = BackpackNormalize::normalize_order(row, None)?;
    Some(ExistingOrder {
        order_id: row.order_id.clone().or_else(|| row.id.clone()).unwrap_or_default(),
        client_order_id: row
            .client_order_id
            .clone()
            .or_else(|| row.client_id.clone())
            .unwrap_or_else(|| intent.client_order_id.clone()),
        intent,
    })
}

fn extract_decimal_field(payload: &Value, names: &[&str]) -> Option<rust_decimal::Decimal> {
    match payload {
        Value::Object(map) => names.iter().find_map(|name| parse_decimal(map.get(*name)?)),
        Value::Array(items) => items.iter().find_map(|item| extract_decimal_field(item, names)),
        _ => None,
    }
}

fn parse_decimal(value: &Value) -> Option<rust_decimal::Decimal> {
    match value {
        Value::String(raw) => raw.parse().ok(),
        Value::Number(num) => num.to_string().parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;
    use serde_json::json;

    use super::*;

    #[test]
    fn extracts_mark_price_from_ticker_shapes() {
        assert_eq!(extract_decimal_field(&json!({"markPrice": "2260.4"}), &["markPrice"]), Some(dec!(2260.4)));
        assert_eq!(extract_decimal_field(&json!({"price": 2260.4}), &["price"]), Some(dec!(2260.4)));
    }
}
