use rust_decimal::Decimal;

use crate::{
    config::AppConfig,
    domain::{OrderSide, Position, RiskDecision, RiskReason},
    precision::short_only_long_flip_epsilon,
    GridMode,
};

#[derive(Debug, Clone)]
pub struct RiskEngine {
    config: AppConfig,
    short_only_long_flip_epsilon: Decimal,
}

impl RiskEngine {
    pub fn new(config: AppConfig) -> Self {
        let epsilon = short_only_long_flip_epsilon(config.order_size);
        Self {
            config,
            short_only_long_flip_epsilon: epsilon,
        }
    }

    pub fn validate_new_order(&self, position: &Position, side: OrderSide, qty: Decimal) -> RiskDecision {
        if self.config.kill_switch {
            return RiskDecision::reject(RiskReason::KillSwitchEnabled);
        }
        if qty <= Decimal::ZERO {
            return RiskDecision::reject(RiskReason::QtyMustBePositive);
        }

        let signed_qty = match side {
            OrderSide::Buy => qty,
            OrderSide::Sell => -qty,
        };
        let projected = position.size + signed_qty;

        if self.config.grid_mode == GridMode::ShortOnly && projected > self.short_only_long_flip_epsilon {
            return RiskDecision::reject(RiskReason::ShortOnlyLongFlipBlocked);
        }
        if projected.abs() > self.config.max_position_abs {
            return RiskDecision::reject(RiskReason::MaxPositionBreached);
        }

        RiskDecision::allow()
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
            spacing_bps: dec!(50),
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
    fn blocks_short_only_long_flip() {
        let engine = RiskEngine::new(cfg());
        let position = Position {
            symbol: "ETH_USDC_PERP".into(),
            size: dec!(-0.003),
            entry_price: dec!(2260.40),
            unrealized_pnl: Decimal::ZERO,
        };
        let decision = engine.validate_new_order(&position, OrderSide::Buy, dec!(0.006));
        assert_eq!(decision.reason, Some(RiskReason::ShortOnlyLongFlipBlocked));
    }

    #[test]
    fn allows_exact_flatten_with_epsilon() {
        let engine = RiskEngine::new(cfg());
        let position = Position {
            symbol: "ETH_USDC_PERP".into(),
            size: dec!(-0.003),
            entry_price: dec!(2260.40),
            unrealized_pnl: Decimal::ZERO,
        };
        let decision = engine.validate_new_order(&position, OrderSide::Buy, dec!(0.003));
        assert!(decision.ok);
    }
}
