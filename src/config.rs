use std::{env, path::PathBuf, str::FromStr, time::Duration};

use anyhow::{anyhow, Result};
use rust_decimal::Decimal;

use crate::model::GridMode;

#[derive(Debug, Clone)]
pub struct Config {
    pub symbol: String,
    pub levels: usize,
    pub spacing_bps: Decimal,
    pub order_size: Decimal,
    pub max_position_abs: Decimal,
    pub grid_mode: GridMode,
    pub grid_active_levels: usize,
    pub grid_short_bias_sell_ratio: usize,
    pub grid_min_price: Option<Decimal>,
    pub grid_max_price: Option<Decimal>,
    pub kill_switch: bool,
    pub quote_asset: String,
    pub loop_interval: Duration,
    pub run_once: bool,
    pub state_path: PathBuf,
    pub events_path: PathBuf,
    pub cycles_path: PathBuf,
    pub live_enabled: bool,
    pub api_base_url: String,
    pub api_key: String,
    pub api_secret: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let state_path = PathBuf::from(env::var("GRID_RUNTIME_STATE_PATH").unwrap_or_else(|_| "runtime/state.json".to_string()));
        let parent = state_path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("runtime"));
        Ok(Self {
            symbol: env::var("GRID_SYMBOL").unwrap_or_else(|_| "ETH_USDC_PERP".to_string()),
            levels: parse_usize("GRID_LEVELS", 5)?,
            spacing_bps: parse_decimal("GRID_SPACING_BPS", "35")?,
            order_size: parse_decimal("GRID_ORDER_SIZE", "0.003")?,
            max_position_abs: parse_decimal("GRID_MAX_POSITION_ABS", "0.02")?,
            grid_mode: parse_grid_mode(env::var("GRID_MODE").unwrap_or_else(|_| "short_only".to_string()).as_str())?,
            grid_active_levels: parse_usize("GRID_ACTIVE_LEVELS", 5)?,
            grid_short_bias_sell_ratio: parse_usize("GRID_SHORT_BIAS_SELL_RATIO", 3)?,
            grid_min_price: parse_optional_decimal("GRID_MIN_PRICE")?,
            grid_max_price: parse_optional_decimal("GRID_MAX_PRICE")?,
            kill_switch: parse_bool("GRID_KILL_SWITCH", false),
            quote_asset: env::var("GRID_QUOTE_ASSET").unwrap_or_else(|_| "USDC".to_string()),
            loop_interval: Duration::from_millis(parse_u64("GRID_SHADOW_INTERVAL_MS", 15_000)?),
            run_once: parse_bool("GRID_RUN_ONCE", false),
            events_path: parent.join("events.jsonl"),
            cycles_path: parent.join("cycles.jsonl"),
            state_path,
            live_enabled: parse_bool("BACKPACK_ENABLE_LIVE", false),
            ws_enabled: parse_bool("BACKPACK_ENABLE_WS", true),
            ws_url: env::var("BACKPACK_WS_URL").ok().filter(|v| !v.trim().is_empty()),
            api_base_url: env::var("BACKPACK_API_BASE_URL").unwrap_or_else(|_| "https://api.backpack.exchange".to_string()),
            api_key: env::var("BACKPACK_API_KEY").unwrap_or_default(),
            api_secret: env::var("BACKPACK_API_SECRET").unwrap_or_default(),
        })
    }
}

fn parse_bool(name: &str, fallback: bool) -> bool {
    env::var(name)
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(fallback)
}

fn parse_decimal(name: &str, fallback: &str) -> Result<Decimal> {
    Decimal::from_str(&env::var(name).unwrap_or_else(|_| fallback.to_string()))
        .map_err(|_| anyhow!("invalid decimal env {}", name))
}

fn parse_optional_decimal(name: &str) -> Result<Option<Decimal>> {
    match env::var(name) {
        Ok(v) if v.trim().is_empty() => Ok(None),
        Ok(v) => Ok(Some(Decimal::from_str(&v).map_err(|_| anyhow!("invalid decimal env {}", name))?)),
        Err(_) => Ok(None),
    }
}

fn parse_usize(name: &str, fallback: usize) -> Result<usize> {
    let raw = env::var(name).unwrap_or_else(|_| fallback.to_string());
    raw.parse::<usize>().map(|v| v.max(1)).map_err(|_| anyhow!("invalid usize env {}", name))
}

fn parse_u64(name: &str, fallback: u64) -> Result<u64> {
    let raw = env::var(name).unwrap_or_else(|_| fallback.to_string());
    raw.parse::<u64>().map(|v| v.max(1)).map_err(|_| anyhow!("invalid u64 env {}", name))
}

fn parse_grid_mode(raw: &str) -> Result<GridMode> {
    match raw {
        "neutral" => Ok(GridMode::Neutral),
        "short_bias" => Ok(GridMode::ShortBias),
        "short_only" => Ok(GridMode::ShortOnly),
        _ => Err(anyhow!("invalid GRID_MODE")),
    }
}
