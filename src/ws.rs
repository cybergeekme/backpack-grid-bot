use std::{net::TcpStream, sync::mpsc::{self, Receiver}, thread, time::Duration};

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use ed25519_dalek::{Signer, SigningKey};
use rust_decimal::Decimal;
use serde_json::Value;
use tungstenite::{connect, stream::MaybeTlsStream, Message, WebSocket};

use crate::{config::Config, model::{OpenOrder, Position, Side}};

#[derive(Debug, Clone)]
pub struct WsSnapshot {
    pub mark_price: Option<Decimal>,
    pub position: Option<Position>,
    pub open_orders: Option<Vec<OpenOrder>>,
}

#[derive(Debug)]
pub enum WsEvent {
    Snapshot(WsSnapshot),
    Info(String),
    Error(String),
}

pub struct BackpackWsHandle {
    rx: Receiver<WsEvent>,
}

impl BackpackWsHandle {
    pub fn poll_latest(&self) -> Vec<WsEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            out.push(ev);
        }
        out
    }
}

pub fn spawn(config: &Config) -> Result<BackpackWsHandle> {
    let ws_url = config.ws_url.clone().ok_or_else(|| anyhow!("BACKPACK_WS_URL not configured"))?;
    let symbol = config.symbol.clone();
    let api_key = config.api_key.clone();
    let api_secret = config.api_secret.clone();
    let ws_enabled = config.ws_enabled;
    if !ws_enabled {
        return Err(anyhow!("ws disabled"));
    }

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        loop {
            match run_loop(&ws_url, &symbol, &api_key, &api_secret) {
                Ok(()) => {
                    let _ = tx.send(WsEvent::Info("ws loop exited, reconnecting".to_string()));
                }
                Err(err) => {
                    let _ = tx.send(WsEvent::Error(format!("ws error: {err:#}")));
                }
            }
            thread::sleep(Duration::from_secs(3));
        }
    });
    Ok(BackpackWsHandle { rx })
}

fn run_loop(ws_url: &str, symbol: &str, api_key: &str, api_secret: &str) -> Result<()> {
    let (mut socket, _) = connect(ws_url)?;
    subscribe_mark_prices(&mut socket, symbol)?;
    if !api_key.is_empty() && !api_secret.is_empty() {
        subscribe_auth(&mut socket, "orders", symbol, api_key, api_secret)?;
        subscribe_auth(&mut socket, "positions", symbol, api_key, api_secret)?;
    }

    loop {
        let msg = socket.read()?;
        match msg {
            Message::Text(text) => {
                let _ = handle_text(&text);
            }
            Message::Ping(data) => socket.send(Message::Pong(data))?,
            Message::Close(_) => return Ok(()),
            _ => {}
        }
    }
}

fn handle_text(text: &str) -> Result<Option<WsEvent>> {
    let payload: Value = serde_json::from_str(text)?;
    let channel = payload.get("channel").or_else(|| payload.get("stream")).or_else(|| payload.get("topic")).and_then(Value::as_str).unwrap_or("");
    let data = payload.get("data").or_else(|| payload.get("payload")).or_else(|| payload.get("result")).unwrap_or(&payload);

    if channel.to_ascii_lowercase().contains("mark") {
        if let Some(arr) = data.as_array() {
            for row in arr {
                if let Some(mark_price) = parse_decimal(row.get("markPrice").or_else(|| row.get("price"))) {
                    return Ok(Some(WsEvent::Snapshot(WsSnapshot { mark_price: Some(mark_price), position: None, open_orders: None })));
                }
            }
        }
    }

    if channel.to_ascii_lowercase().contains("position") {
        let pos = Position {
            symbol: data.get("symbol").and_then(Value::as_str).unwrap_or_default().to_string(),
            size: parse_decimal(data.get("netQuantity").or_else(|| data.get("quantity"))).unwrap_or(Decimal::ZERO),
            entry_price: parse_decimal(data.get("entryPrice").or_else(|| data.get("averageEntryPrice"))).unwrap_or(Decimal::ZERO),
            unrealized_pnl: parse_decimal(data.get("unrealizedPnl").or_else(|| data.get("pnl"))).unwrap_or(Decimal::ZERO),
        };
        return Ok(Some(WsEvent::Snapshot(WsSnapshot { mark_price: None, position: Some(pos), open_orders: None })));
    }

    if channel.to_ascii_lowercase().contains("order") {
        if let Some(order) = normalize_open_order(data) {
            return Ok(Some(WsEvent::Snapshot(WsSnapshot { mark_price: None, position: None, open_orders: Some(vec![order]) })));
        }
    }

    Ok(None)
}

fn subscribe_mark_prices(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>, symbol: &str) -> Result<()> {
    let payload = serde_json::json!({ "method": "SUBSCRIBE", "params": { "channel": "markPrices", "symbol": symbol } });
    socket.send(Message::Text(payload.to_string().into()))?;
    Ok(())
}

fn subscribe_auth(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>, channel: &str, symbol: &str, api_key: &str, api_secret: &str) -> Result<()> {
    let timestamp = chrono::Utc::now().timestamp_millis();
    let nonce = uuid::Uuid::new_v4().to_string();
    let instruction = format!("subscribe:{channel}");
    let signing_key = decode_signing_key(api_secret)?;
    let signature = BASE64.encode(signing_key.sign(format!("{instruction}:{timestamp}:{nonce}").as_bytes()).to_bytes());
    let payload = serde_json::json!({
        "method": "SUBSCRIBE",
        "params": {
            "channel": channel,
            "symbol": symbol,
            "apiKey": api_key,
            "timestamp": timestamp,
            "nonce": nonce,
            "signature": signature
        }
    });
    socket.send(Message::Text(payload.to_string().into()))?;
    Ok(())
}

fn decode_signing_key(secret: &str) -> Result<SigningKey> {
    let raw = BASE64.decode(secret.as_bytes())?;
    let seed: [u8; 32] = raw.get(0..32).ok_or_else(|| anyhow!("BACKPACK_API_SECRET seed too short"))?.try_into().map_err(|_| anyhow!("invalid secret seed length"))?;
    Ok(SigningKey::from_bytes(&seed))
}

fn parse_decimal(value: Option<&Value>) -> Option<Decimal> {
    match value? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.to_string().parse().ok(),
        _ => None,
    }
}

fn normalize_open_order(data: &Value) -> Option<OpenOrder> {
    let status = data.get("status").and_then(Value::as_str).unwrap_or("Open");
    if !matches!(status, "Open" | "New" | "Accepted" | "open" | "new" | "accepted") { return None; }
    Some(OpenOrder {
        order_id: value_to_string(data.get("orderId").or_else(|| data.get("id"))?),
        client_order_id: data.get("clientOrderId").or_else(|| data.get("clientId")).map(value_to_string).unwrap_or_default(),
        symbol: data.get("symbol").and_then(Value::as_str).unwrap_or_default().to_string(),
        side: match data.get("side").and_then(Value::as_str).unwrap_or("Bid") { "Ask" | "sell" | "Sell" => Side::Sell, _ => Side::Buy },
        price: parse_decimal(data.get("price")).unwrap_or(Decimal::ZERO),
        qty: parse_decimal(data.get("quantity").or_else(|| data.get("qty"))).unwrap_or(Decimal::ZERO),
        reduce_only: data.get("reduceOnly").and_then(Value::as_bool).unwrap_or(false),
        post_only: data.get("postOnly").and_then(Value::as_bool).unwrap_or(false),
        status: status.to_string(),
    })
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => v.to_string(),
    }
}
