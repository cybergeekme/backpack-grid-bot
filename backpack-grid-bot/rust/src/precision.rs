use rust_decimal::Decimal;

pub fn order_precision_decimals(order_size: Decimal) -> u32 {
    normalize_decimal(order_size).scale()
}

pub fn price_precision_decimals(price: Decimal) -> u32 {
    normalize_decimal(price).scale().max(2)
}

pub fn normalize_order_qty(raw_qty: Decimal, order_size: Decimal) -> Decimal {
    normalize_to_scale(raw_qty, order_precision_decimals(order_size))
}

pub fn normalize_position_size(raw_size: Decimal, order_size: Decimal) -> Decimal {
    normalize_to_scale(raw_size, order_precision_decimals(order_size))
}

pub fn short_only_long_flip_epsilon(order_size: Decimal) -> Decimal {
    let decimals = order_precision_decimals(order_size);
    Decimal::new(1, decimals + 3)
}

pub fn normalize_to_scale(value: Decimal, scale: u32) -> Decimal {
    value.round_dp(scale).normalize()
}

pub fn normalize_decimal(value: Decimal) -> Decimal {
    value.normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn qty_normalization_matches_order_size_scale() {
        assert_eq!(normalize_order_qty(dec!(0.0030000000000000005), dec!(0.003)), dec!(0.003));
        assert_eq!(normalize_position_size(dec!(-0.009000000000000001), dec!(0.003)), dec!(-0.009));
    }

    #[test]
    fn epsilon_is_tighter_than_order_precision() {
        assert_eq!(short_only_long_flip_epsilon(dec!(0.003)), dec!(0.000001));
        assert_eq!(short_only_long_flip_epsilon(dec!(1)), dec!(0.001));
    }
}
