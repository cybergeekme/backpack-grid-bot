use std::collections::{BTreeMap, BTreeSet};

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use ed25519_dalek::{Signer, SigningKey};
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, USER_AGENT};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::{config::Config, model::{DesiredOrder, OpenOrder, Position, Side}};

#[derive(Debug, Clone)]
pub struct BackpackClient {
    http: Client,
    base_url: String,
    api_key: String,
    signing_key: SigningKey,
    live_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct SyncResult {
    pub final_orders: Vec<OpenOrder>,
    pub place_orders: Vec<DesiredOrder>,
    pub cancel_orders: Vec<OpenOrder>,
    pub matched_orders: usize,
}

impl BackpackClient {
    pub fn new(config: &Config) -> Result<Self> {
        if config.api_key.is_empty() || config.api_secret.is_empty() {
            return Err(anyhow!("missing BACKPACK_API_KEY/BACKPACK_API_SECRET"));
        }
        let secret = BASE64.decode(config.api_secret.as_bytes()).context("decode BACKPACK_API_SECRET")?;
        let seed: [u8; 32] = secret.get(0..32).ok_or_else(|| anyhow!("BACKPACK_API_SECRET seed too short"))?.try_into().map_err(|_| anyhow!("invalid secret seed length"))?;
        Ok(Self {
            http: Client::builder().timeout(std::time::Duration::from_secs(15)).build()?,
            base_url: config.api_base_url.trim_end_matches('/').to_string(),
            api_key: config.api_key.clone(),
            signing_key: SigningKey::from_bytes(&seed),
            live_enabled: config.live_enabled,
        })
    }

    pub fn fetch_mark_price(&self, symbol: &str) -> Result<Decimal> {
        let v = self.public_get("/api/v1/markPrices", &[])?;
        let rows = v.as_array().ok_or_else(|| anyhow!("invalid markPrices payload"))?;
        let row = rows.iter().find(|r| r.get("symbol").and_then(Value::as_str) == Some(symbol)).ok_or_else(|| anyhow!("symbol not found in markPrices"))?;
        parse_decimal(row.get("markPrice").or_else(|| row.get("price"))).ok_or_else(|| anyhow!("mark price missing"))
    }

    pub fn fetch_position(&self, symbol: &str) -> Result<Position> {
        let v = self.signed_get("/api/v1/position", &[(&"symbol", symbol)])?;
        let row = if let Some(arr) = v.as_array() {
            arr.iter().find(|r| r.get("symbol").and_then(Value::as_str) == Some(symbol)).cloned()
        } else {
            Some(v)
        };
        let Some(row) = row else { return Ok(Position::flat(symbol)); };
        Ok(Position {
            symbol: symbol.to_string(),
            size: parse_decimal(row.get("netQuantity").or_else(|| row.get("net_quantity")).or_else(|| row.get("quantity"))).unwrap_or(Decimal::ZERO),
            entry_price: parse_decimal(row.get("entryPrice").or_else(|| row.get("entry_price")).or_else(|| row.get("averageEntryPrice"))).unwrap_or(Decimal::ZERO),
            unrealized_pnl: parse_decimal(row.get("unrealizedPnl").or_else(|| row.get("unrealized_pnl")).or_else(|| row.get("pnl"))).unwrap_or(Decimal::ZERO),
        })
    }

    pub fn fetch_open_orders(&self, symbol: &str) -> Result<Vec<OpenOrder>> {
        let v = self.signed_get("/api/v1/orders", &[(&"symbol", symbol)])?;
        let rows = v.as_array().ok_or_else(|| anyhow!("invalid orders payload"))?;
        Ok(rows.iter().filter_map(|row| normalize_open_order(row, symbol)).collect())
    }

    pub fn sync_orders(&self, symbol: &str, desired: &[DesiredOrder], existing: &[OpenOrder]) -> Result<SyncResult> {
        let desired_map = desired.iter().cloned().map(|o| (order_key_desired(&o), o)).collect::<BTreeMap<_, _>>();
        let existing_map = existing.iter().cloned().map(|o| (order_key_open(&o), o)).collect::<BTreeMap<_, _>>();
        let desired_keys = desired_map.keys().cloned().collect::<BTreeSet<_>>();
        let existing_keys = existing_map.keys().cloned().collect::<BTreeSet<_>>();

        let place_orders = desired_keys.difference(&existing_keys).filter_map(|k| desired_map.get(k).cloned()).collect::<Vec<_>>();
        let cancel_orders = existing_keys.difference(&desired_keys).filter_map(|k| existing_map.get(k).cloned()).collect::<Vec<_>>();
        let matched_orders = existing_keys.intersection(&desired_keys).count();

        if self.live_enabled {
            for order in &cancel_orders {
                self.cancel_order(symbol, &order.order_id)?;
            }
            for order in &place_orders {
                self.place_order(order)?;
            }
        }

        let final_orders = if self.live_enabled {
            self.fetch_open_orders(symbol)?
        } else {
            desired.iter().map(|o| OpenOrder {
                order_id: format!("dryrun:{}", o.client_order_id),
                client_order_id: o.client_order_id.clone(),
                symbol: o.symbol.clone(),
                side: o.side,
                price: o.price,
                qty: o.qty,
                reduce_only: o.reduce_only,
                post_only: o.post_only,
                status: "open".to_string(),
            }).collect()
        };

        Ok(SyncResult { final_orders, place_orders, cancel_orders, matched_orders })
    }

    fn place_order(&self, order: &DesiredOrder) -> Result<()> {
        let side = match order.side { Side::Buy => "Bid", Side::Sell => "Ask" };
        let client_id = numeric_client_id(&order.client_order_id);
        let payload = serde_json::json!({
            "symbol": order.symbol,
            "side": side,
            "orderType": "Limit",
            "price": order.price.to_string(),
            "quantity": order.qty.to_string(),
            "postOnly": order.post_only,
            "reduceOnly": order.reduce_only,
            "timeInForce": "GTC",
            "clientId": client_id,
        });
        let _ = self.signed_send("POST", "/api/v1/order", "orderExecute", &payload)?;
        Ok(())
    }

    fn cancel_order(&self, symbol: &str, order_id: &str) -> Result<()> {
        let payload = serde_json::json!({ "symbol": symbol, "orderId": order_id });
        let _ = self.signed_send("DELETE", "/api/v1/order", "orderCancel", &payload)?;
        Ok(())
    }

    fn public_get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.http.get(url).query(query).send()?.error_for_status()?;
        Ok(response.json()?)
    }

    fn signed_get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let mut params = BTreeMap::new();
        for (k, v) in query { params.insert((*k).to_string(), (*v).to_string()); }
        let request = self.http.get(format!("{}{}", self.base_url, path)).query(query);
        let response = self.with_signed_headers(request, instruction_for(path), &params)?.send()?.error_for_status()?;
        Ok(response.json()?)
    }

    fn signed_send(&self, method: &str, path: &str, instruction: &str, payload: &Value) -> Result<Value> {
        let params = json_to_btree(payload);
        let request = match method {
            "POST" => self.http.post(format!("{}{}", self.base_url, path)).json(payload),
            "DELETE" => self.http.delete(format!("{}{}", self.base_url, path)).json(payload),
            _ => return Err(anyhow!("unsupported method")),
        };
        let response = self.with_signed_headers(request, instruction, &params)?.send()?.error_for_status()?;
        Ok(response.json()?)
    }

    fn with_signed_headers(&self, request: RequestBuilder, instruction: &str, params: &BTreeMap<String, String>) -> Result<RequestBuilder> {
        let timestamp = chrono::Utc::now().timestamp_millis();
        let window = "5000".to_string();
        let signature_payload = build_signature_payload(instruction, params, timestamp, &window);
        let signature = BASE64.encode(self.signing_key.sign(signature_payload.as_bytes()).to_bytes());
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("backpack-grid-bot/0.2"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert("X-API-Key", HeaderValue::from_str(&self.api_key)?);
        headers.insert("X-Timestamp", HeaderValue::from_str(&timestamp.to_string())?);
        headers.insert("X-Window", HeaderValue::from_str(&window)?);
        headers.insert("X-Signature", HeaderValue::from_str(&signature)?);
        Ok(request.headers(headers))
    }
}

fn build_signature_payload(instruction: &str, params: &BTreeMap<String, String>, timestamp: i64, window: &str) -> String {
    let mut parts = vec![format!("instruction={instruction}")];
    for (k, v) in params { if !v.is_empty() { parts.push(format!("{}={}", k, v)); } }
    parts.push(format!("timestamp={timestamp}"));
    parts.push(format!("window={window}"));
    parts.join("&")
}

fn instruction_for(path: &str) -> &'static str {
    match path {
        "/api/v1/position" => "positionQuery",
        "/api/v1/orders" => "orderQueryAll",
        "/api/v1/order" => "orderExecute",
        "/api/v1/capital/collateral" => "collateralQuery",
        _ => "accountQuery",
    }
}

fn normalize_open_order(row: &Value, symbol: &str) -> Option<OpenOrder> {
    let status = row.get("status").and_then(Value::as_str).unwrap_or("Open");
    if !matches!(status, "Open" | "New" | "Accepted" | "open" | "new" | "accepted") { return None; }
    Some(OpenOrder {
        order_id: row.get("orderId").or_else(|| row.get("id")).map(value_to_string).unwrap_or_default(),
        client_order_id: row.get("clientOrderId").or_else(|| row.get("client_id")).or_else(|| row.get("clientId")).map(value_to_string).unwrap_or_default(),
        symbol: row.get("symbol").and_then(Value::as_str).unwrap_or(symbol).to_string(),
        side: match row.get("side").and_then(Value::as_str).unwrap_or("Bid") { "Ask" | "sell" | "Sell" => Side::Sell, _ => Side::Buy },
        price: parse_decimal(row.get("price")).unwrap_or(Decimal::ZERO),
        qty: parse_decimal(row.get("quantity").or_else(|| row.get("qty"))).unwrap_or(Decimal::ZERO),
        reduce_only: row.get("reduceOnly").and_then(Value::as_bool).unwrap_or(false),
        post_only: row.get("postOnly").and_then(Value::as_bool).unwrap_or(false),
        status: status.to_string(),
    })
}

fn json_to_btree(payload: &Value) -> BTreeMap<String, String> {
    payload.as_object().map(|m| {
        m.iter().filter_map(|(k, v)| {
            if v.is_null() { None } else { Some((k.clone(), value_to_string(v))) }
        }).collect()
    }).unwrap_or_default()
}

fn parse_decimal(value: Option<&Value>) -> Option<Decimal> {
    match value? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.to_string().parse().ok(),
        _ => None,
    }
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "true".to_string() } else { "false".to_string() },
        Value::Number(n) => n.to_string(),
        _ => v.to_string(),
    }
}

fn numeric_client_id(source: &str) -> u32 {
    let digest = md5::compute(source.as_bytes());
    let value = u32::from_be_bytes([digest.0[0], digest.0[1], digest.0[2], digest.0[3]]);
    value.max(1)
}

fn order_key_desired(order: &DesiredOrder) -> String {
    format!("{:?}:{}:{}:{}:{}", order.side, order.price.normalize(), order.qty.normalize(), order.reduce_only, order.post_only)
}

fn order_key_open(order: &OpenOrder) -> String {
    format!("{:?}:{}:{}:{}:{}", order.side, order.price.normalize(), order.qty.normalize(), order.reduce_only, order.post_only)
}
