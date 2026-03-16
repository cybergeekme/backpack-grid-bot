use std::env;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use crate::domain::GridMode;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    pub symbol: String,
    pub levels: usize,
    pub spacing_bps: Decimal,
    pub order_size: Decimal,
    pub max_position_abs: Decimal,
    pub grid_mode: GridMode,
    pub grid_active_levels: usize,
    pub grid_short_bias_sell_ratio: Decimal,
    pub grid_min_price: Option<Decimal>,
    pub grid_max_price: Option<Decimal>,
    pub leverage: Decimal,
    pub quote_asset: String,
    pub kill_switch: bool,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid decimal env {name}={raw}")]
    InvalidDecimal { name: &'static str, raw: String },
    #[error("invalid grid mode env GRID_MODE={0}")]
    InvalidGridMode(String),
}

impl AppConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            symbol: env::var("GRID_SYMBOL").unwrap_or_else(|_| "ETH_USDC_PERP".to_string()),
            levels: num_usize("GRID_LEVELS", 3)?,
            spacing_bps: num_decimal("GRID_SPACING_BPS", dec("50"))?,
            order_size: num_decimal("GRID_ORDER_SIZE", dec("0.01"))?,
            max_position_abs: num_decimal("GRID_MAX_POSITION_ABS", dec("0.05"))?,
            grid_mode: grid_mode()?,
            grid_active_levels: num_usize("GRID_ACTIVE_LEVELS", 3)?,
            grid_short_bias_sell_ratio: num_decimal("GRID_SHORT_BIAS_SELL_RATIO", dec("3"))?,
            grid_min_price: optional_decimal("GRID_MIN_PRICE")?,
            grid_max_price: optional_decimal("GRID_MAX_PRICE")?,
            leverage: num_decimal("GRID_LEVERAGE", dec("1"))?,
            quote_asset: env::var("GRID_QUOTE_ASSET").unwrap_or_else(|_| "USDC".to_string()),
            kill_switch: bool_flag("GRID_KILL_SWITCH", false),
        })
    }
}

fn num_decimal(name: &'static str, fallback: Decimal) -> Result<Decimal, ConfigError> {
    match env::var(name) {
        Ok(raw) => raw.parse::<Decimal>().map_err(|_| ConfigError::InvalidDecimal {
            name,
            raw,
        }),
        Err(_) => Ok(fallback),
    }
}

fn optional_decimal(name: &'static str) -> Result<Option<Decimal>, ConfigError> {
    match env::var(name) {
        Ok(raw) => raw
            .parse::<Decimal>()
            .map(Some)
            .map_err(|_| ConfigError::InvalidDecimal { name, raw }),
        Err(_) => Ok(None),
    }
}

fn num_usize(name: &'static str, fallback: usize) -> Result<usize, ConfigError> {
    let value = num_decimal(name, Decimal::from(fallback as u64))?;
    Ok(value.floor().to_u64().unwrap_or(fallback as u64).max(1) as usize)
}

fn bool_flag(name: &'static str, fallback: bool) -> bool {
    match env::var(name) {
        Ok(raw) => matches!(raw.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => fallback,
    }
}

fn grid_mode() -> Result<GridMode, ConfigError> {
    match env::var("GRID_MODE") {
        Ok(raw) => match raw.as_str() {
            "neutral" => Ok(GridMode::Neutral),
            "short_bias" => Ok(GridMode::ShortBias),
            "short_only" => Ok(GridMode::ShortOnly),
            _ => Err(ConfigError::InvalidGridMode(raw)),
        },
        Err(_) => Ok(GridMode::Neutral),
    }
}

fn dec(value: &'static str) -> Decimal {
    value.parse::<Decimal>().expect("static decimal literal")
}

trait DecimalToU64 {
    fn to_u64(&self) -> Option<u64>;
}

impl DecimalToU64 for Decimal {
    fn to_u64(&self) -> Option<u64> {
        self.trunc().to_string().parse::<u64>().ok()
    }
}
