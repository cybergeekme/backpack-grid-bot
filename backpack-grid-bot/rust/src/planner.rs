use rust_decimal::Decimal;

use crate::{
    config::AppConfig,
    domain::{GridLevel, OrderIntent, OrderSide, OrderType, PlannerOutput, Position, RiskReason},
    precision::{normalize_order_qty, normalize_position_size},
    risk::RiskEngine,
    GridMode,
};

#[derive(Debug, Clone)]
pub struct GridPlanner {
    config: AppConfig,
    risk: RiskEngine,
}

impl GridPlanner {
    pub fn new(config: AppConfig) -> Self {
        let risk = RiskEngine::new(config.clone());
        Self { config, risk }
    }

    pub fn build_grid(&self, mid_price: Decimal) -> Vec<GridLevel> {
        if let Some(levels) = self.build_bounded_grid(mid_price) {
            return levels;
        }

        let mut levels = Vec::new();
        for i in 1..=self.config.levels {
            let ratio = (self.config.spacing_bps * Decimal::from(i as u64)) / Decimal::from(10_000u64);
            levels.push(GridLevel {
                index: i,
                side: OrderSide::Buy,
                price: round_price(mid_price * (Decimal::ONE - ratio)),
                qty: self.config.order_size,
            });
            levels.push(GridLevel {
                index: i,
                side: OrderSide::Sell,
                price: round_price(mid_price * (Decimal::ONE + ratio)),
                qty: self.config.order_size,
            });
        }
        levels.sort_by(|a, b| a.price.cmp(&b.price));
        levels
    }

    pub fn select_active_levels(&self, levels: &[GridLevel], mid_price: Decimal, position: Option<&Position>) -> Vec<GridLevel> {
        let mut sorted = levels.to_vec();
        sorted.sort_by(|a, b| {
            let da = (a.price - mid_price).abs();
            let db = (b.price - mid_price).abs();
            da.cmp(&db)
        });
        let sells: Vec<GridLevel> = sorted.iter().filter(|level| level.side == OrderSide::Sell).cloned().collect();
        let buys: Vec<GridLevel> = sorted.iter().filter(|level| level.side == OrderSide::Buy).cloned().collect();
        let per_side = self.config.grid_active_levels;

        let mut out = match self.config.grid_mode {
            GridMode::ShortOnly => {
                let sell_levels = sells.into_iter().take(per_side);
                let reduce_buy_slots = match position {
                    Some(position) if position.size < Decimal::ZERO => {
                        let slots = ((-position.size) / self.config.order_size).ceil();
                        let slots = slots.to_string().parse::<usize>().unwrap_or(0);
                        buys.len().min(per_side).min(slots)
                    }
                    _ => 0,
                };
                let reduce_buy_levels = buys.into_iter().take(reduce_buy_slots);
                reduce_buy_levels.chain(sell_levels).collect::<Vec<_>>()
            }
            GridMode::ShortBias => {
                let sell_count = ((Decimal::from(per_side as u64) * self.config.grid_short_bias_sell_ratio)
                    .round()
                    .to_string()
                    .parse::<usize>()
                    .unwrap_or(per_side))
                    .max(1)
                    .min(sells.len());
                buys.into_iter().take(per_side).chain(sells.into_iter().take(sell_count)).collect::<Vec<_>>()
            }
            GridMode::Neutral => buys.into_iter().take(per_side).chain(sells.into_iter().take(per_side)).collect::<Vec<_>>(),
        };

        out.sort_by(|a, b| a.price.cmp(&b.price));
        out
    }

    pub fn plan_orders(&self, mid_price: Decimal, position: Option<&Position>) -> PlannerOutput {
        let levels = self.build_grid(mid_price);
        let active_levels = self.select_active_levels(&levels, mid_price, position);
        let position = position.cloned().unwrap_or_else(|| Position::flat(self.config.symbol.clone()));
        let mut desired_orders = Vec::new();
        let mut rejected_levels = Vec::new();
        let mut planned_position = normalize_position_size(position.size, self.config.order_size);

        for level in &active_levels {
            let is_short_only_reduce_buy = self.config.grid_mode == GridMode::ShortOnly && level.side == OrderSide::Buy;
            let max_reducible_qty = if is_short_only_reduce_buy {
                Decimal::ZERO.max(-planned_position)
            } else {
                level.qty
            };
            let raw_qty = if is_short_only_reduce_buy {
                level.qty.min(max_reducible_qty)
            } else {
                level.qty
            };
            let qty = normalize_order_qty(raw_qty, self.config.order_size);
            if qty <= Decimal::ZERO {
                continue;
            }

            let projected = Position {
                symbol: position.symbol.clone(),
                size: planned_position,
                entry_price: position.entry_price,
                unrealized_pnl: position.unrealized_pnl,
            };
            let decision = self.risk.validate_new_order(&projected, level.side, qty);
            if !decision.ok {
                rejected_levels.push((level.clone(), decision.reason.unwrap_or(RiskReason::QtyMustBePositive)));
                continue;
            }

            desired_orders.push(OrderIntent {
                client_order_id: format!("grid-{}-{}-{}", side_label(level.side), level.index, format_price_key(level.price)),
                symbol: self.config.symbol.clone(),
                side: level.side,
                order_type: OrderType::Limit,
                price: level.price,
                qty,
                reduce_only: is_short_only_reduce_buy,
                post_only: true,
            });

            planned_position = normalize_position_size(
                planned_position
                    + match level.side {
                        OrderSide::Buy => qty,
                        OrderSide::Sell => -qty,
                    },
                self.config.order_size,
            );
        }

        PlannerOutput {
            active_levels,
            desired_orders,
            rejected_levels,
            projected_position_after_orders: planned_position,
        }
    }

    fn build_bounded_grid(&self, mid_price: Decimal) -> Option<Vec<GridLevel>> {
        let min_price = self.config.grid_min_price?;
        let max_price = self.config.grid_max_price?;
        if !(min_price > Decimal::ZERO && max_price > min_price) {
            return None;
        }
        let slices = self.config.levels + 1;
        let step = (max_price - min_price) / Decimal::from(slices as u64);
        let mut levels = Vec::new();
        for i in 1..=self.config.levels {
            let price = round_price(min_price + step * Decimal::from(i as u64));
            if price == round_price(mid_price) {
                continue;
            }
            levels.push(GridLevel {
                index: i,
                side: if price < mid_price { OrderSide::Buy } else { OrderSide::Sell },
                price,
                qty: self.config.order_size,
            });
        }
        levels.sort_by(|a, b| a.price.cmp(&b.price));
        Some(levels)
    }
}

fn round_price(value: Decimal) -> Decimal {
    value.round_dp(2).normalize()
}

fn format_price_key(price: Decimal) -> String {
    (price * Decimal::from(100u64)).round().to_string().replace('.', "")
}

fn side_label(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "buy",
        OrderSide::Sell => "sell",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn cfg() -> AppConfig {
        AppConfig {
            symbol: "ETH_USDC_PERP".into(),
            levels: 5,
            spacing_bps: dec!(35),
            order_size: dec!(0.003),
            max_position_abs: dec!(0.02),
            grid_mode: GridMode::ShortOnly,
            grid_active_levels: 5,
            grid_short_bias_sell_ratio: dec!(3),
            grid_min_price: None,
            grid_max_price: None,
            leverage: dec!(10),
            quote_asset: "USDC".into(),
            kill_switch: false,
        }
    }

    #[test]
    fn short_only_planner_only_places_reduce_buys_when_short_exists() {
        let planner = GridPlanner::new(cfg());
        let position = Position {
            symbol: "ETH_USDC_PERP".into(),
            size: dec!(-0.006),
            entry_price: dec!(2262.38),
            unrealized_pnl: Decimal::ZERO,
        };
        let out = planner.plan_orders(dec!(2260.40), Some(&position));
        let buys = out.desired_orders.iter().filter(|o| o.side == OrderSide::Buy).collect::<Vec<_>>();
        let sells = out.desired_orders.iter().filter(|o| o.side == OrderSide::Sell).collect::<Vec<_>>();
        assert_eq!(buys.len(), 2);
        assert!(buys.iter().all(|o| o.reduce_only));
        assert_eq!(sells.len(), 5);
    }

    #[test]
    fn short_only_planner_does_not_emit_buys_when_flat() {
        let planner = GridPlanner::new(cfg());
        let out = planner.plan_orders(dec!(2260.40), Some(&Position::flat("ETH_USDC_PERP")));
        assert!(out.desired_orders.iter().all(|o| o.side == OrderSide::Sell));
    }
}
