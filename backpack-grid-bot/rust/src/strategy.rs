use rust_decimal::Decimal;

use crate::{config::Config, model::{DesiredOrder, GridMode, Position, Side}};

pub fn build_orders(config: &Config, mark_price: Decimal, position: &Position) -> (Vec<DesiredOrder>, bool, Option<String>, Vec<String>) {
    let mut issues = Vec::new();
    if config.kill_switch {
        return (Vec::new(), true, Some("kill_switch".to_string()), issues);
    }
    if config.grid_min_price.is_some_and(|v| mark_price < v) {
        return (Vec::new(), true, Some("below_grid_min_price".to_string()), issues);
    }
    if config.grid_max_price.is_some_and(|v| mark_price > v) {
        return (Vec::new(), true, Some("above_grid_max_price".to_string()), issues);
    }

    let mut orders = Vec::new();
    let mut projected = position.size;
    let spacing = mark_price * config.spacing_bps / Decimal::from(10_000u32);
    let buy_count = match config.grid_mode {
        GridMode::ShortOnly => if position.size < Decimal::ZERO { config.grid_active_levels.min(config.levels) } else { 0 },
        GridMode::ShortBias => config.grid_active_levels.min(config.levels),
        GridMode::Neutral => config.grid_active_levels.min(config.levels),
    };
    let sell_count = match config.grid_mode {
        GridMode::ShortOnly => config.grid_active_levels.min(config.levels),
        GridMode::ShortBias => (config.grid_active_levels * config.grid_short_bias_sell_ratio).min(config.levels),
        GridMode::Neutral => config.grid_active_levels.min(config.levels),
    };

    for i in 1..=buy_count {
        let price = normalize_price(mark_price - spacing * Decimal::from(i as u64));
        let mut qty = config.order_size;
        let reduce_only = matches!(config.grid_mode, GridMode::ShortOnly);
        if reduce_only {
            let max_reducible = (-projected).max(Decimal::ZERO);
            if max_reducible <= Decimal::ZERO { continue; }
            qty = qty.min(max_reducible);
        }
        qty = normalize_qty(qty, config.order_size);
        if qty <= Decimal::ZERO { continue; }
        let next = projected + qty;
        if matches!(config.grid_mode, GridMode::ShortOnly) && next > Decimal::ZERO {
            issues.push("short_only_long_flip_blocked".to_string());
            continue;
        }
        orders.push(DesiredOrder {
            client_order_id: client_id(Side::Buy, i, price),
            symbol: config.symbol.clone(),
            side: Side::Buy,
            price,
            qty,
            reduce_only,
            post_only: true,
        });
        projected = next;
    }

    for i in 1..=sell_count {
        let price = normalize_price(mark_price + spacing * Decimal::from(i as u64));
        let qty = normalize_qty(config.order_size, config.order_size);
        if qty <= Decimal::ZERO { continue; }
        let next = projected - qty;
        if next.abs() > config.max_position_abs {
            issues.push("max_position_breached".to_string());
            continue;
        }
        orders.push(DesiredOrder {
            client_order_id: client_id(Side::Sell, i, price),
            symbol: config.symbol.clone(),
            side: Side::Sell,
            price,
            qty,
            reduce_only: false,
            post_only: true,
        });
        projected = next;
    }

    orders.sort_by(|a, b| a.price.cmp(&b.price));
    (orders, false, None, issues)
}

fn client_id(side: Side, level: usize, price: Decimal) -> String {
    let side_str = match side { Side::Buy => "buy", Side::Sell => "sell" };
    format!("grid-{}-{}-{}", side_str, level, price.normalize())
}

pub fn normalize_qty(value: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO || value <= Decimal::ZERO { return Decimal::ZERO; }
    let scale = step.normalize().scale();
    value.round_dp(scale)
}

fn normalize_price(value: Decimal) -> Decimal {
    let scale = value.normalize().scale().max(2);
    value.round_dp(scale)
}

trait DecimalExt {
    fn scale(&self) -> u32;
}

impl DecimalExt for Decimal {
    fn scale(&self) -> u32 {
        self.normalize().to_string().split('.').nth(1).map(|s| s.len() as u32).unwrap_or(0)
    }
}
