use rust_decimal::Decimal;

use crate::{
    execution::{ExistingOrder, ReconciliationDiff},
    OrderIntent, OrderSide, Position,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillSource {
    AdapterEvent,
    ReconciliationFallback,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FillMetrics {
    pub fee: Decimal,
    pub realized_pnl: Decimal,
    pub position_after: Decimal,
    pub entry_price_after: Decimal,
    pub closed_qty: Decimal,
    pub opening_fill: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyntheticFill {
    pub source: FillSource,
    pub symbol: String,
    pub side: OrderSide,
    pub qty: Decimal,
    pub price: Option<Decimal>,
    pub order_id: Option<String>,
    pub client_order_id: Option<String>,
    pub fee: Decimal,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReconcileSnapshot {
    pub position: Option<Position>,
    pub orders: Vec<ExistingOrder>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReconcileOutcome {
    pub diff: ReconciliationDiff,
    pub synthetic_fill: Option<SyntheticFill>,
}

#[derive(Debug, Clone)]
pub struct ReconcileEngine {
    symbol: String,
    qty_epsilon: Decimal,
}

impl ReconcileEngine {
    pub fn new(symbol: impl Into<String>, qty_epsilon: Decimal) -> Self {
        Self {
            symbol: symbol.into(),
            qty_epsilon,
        }
    }

    pub fn reconcile(
        &self,
        previous: &ReconcileSnapshot,
        current: &ReconcileSnapshot,
        desired_orders: &[OrderIntent],
    ) -> ReconcileOutcome {
        let diff = self.diff_orders(&current.orders, desired_orders);
        let synthetic_fill = self.infer_fill_from_reconciliation(
            previous.position.as_ref(),
            current.position.as_ref(),
            &previous.orders,
            &current.orders,
        );
        ReconcileOutcome { diff, synthetic_fill }
    }

    pub fn diff_orders(&self, current_orders: &[ExistingOrder], desired_orders: &[OrderIntent]) -> ReconciliationDiff {
        let current_keys = current_orders
            .iter()
            .map(|order| key_from_intent(&order.intent))
            .collect::<std::collections::BTreeSet<_>>();
        let desired_map = desired_orders
            .iter()
            .cloned()
            .map(|intent| (key_from_intent(&intent), intent))
            .collect::<std::collections::BTreeMap<_, _>>();
        let desired_keys = desired_map.keys().cloned().collect::<std::collections::BTreeSet<_>>();

        let missing_on_exchange = desired_keys
            .difference(&current_keys)
            .filter_map(|key| desired_map.get(key).cloned())
            .collect::<Vec<_>>();

        let unexpected_on_exchange = current_orders
            .iter()
            .filter(|order| !desired_keys.contains(&key_from_intent(&order.intent)))
            .cloned()
            .collect::<Vec<_>>();

        let matched = desired_orders.len().saturating_sub(missing_on_exchange.len());

        ReconciliationDiff {
            missing_on_exchange,
            unexpected_on_exchange,
            matched,
        }
    }

    pub fn infer_fill_from_reconciliation(
        &self,
        previous_position: Option<&Position>,
        current_position: Option<&Position>,
        previous_orders: &[ExistingOrder],
        current_orders: &[ExistingOrder],
    ) -> Option<SyntheticFill> {
        let before = previous_position.map(|p| p.size).unwrap_or(Decimal::ZERO);
        let after = current_position.map(|p| p.size).unwrap_or(Decimal::ZERO);
        let delta = after - before;
        if delta.abs() <= self.qty_epsilon {
            return None;
        }

        let side = if delta > Decimal::ZERO {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        };
        let qty = delta.abs();
        let current_ids = current_orders
            .iter()
            .map(|order| order.order_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let missing_candidates = previous_orders
            .iter()
            .filter(|order| !current_ids.contains(&order.order_id) && order.intent.side == side)
            .cloned()
            .collect::<Vec<_>>();

        let candidate = missing_candidates
            .iter()
            .find(|order| (order.intent.qty - qty).abs() <= self.qty_epsilon)
            .cloned()
            .or_else(|| missing_candidates.into_iter().next());

        Some(SyntheticFill {
            source: FillSource::ReconciliationFallback,
            symbol: self.symbol.clone(),
            side,
            qty,
            price: candidate.as_ref().map(|order| order.intent.price),
            order_id: candidate.as_ref().map(|order| order.order_id.clone()),
            client_order_id: candidate.as_ref().map(|order| order.client_order_id.clone()),
            fee: Decimal::ZERO,
        })
    }

    pub fn estimate_fill_metrics(&self, position_before: Option<&Position>, fill: &SyntheticFill) -> FillMetrics {
        let fee = fill.fee;
        let before_size = position_before.map(|p| p.size).unwrap_or(Decimal::ZERO);
        let before_entry = position_before.map(|p| p.entry_price).unwrap_or(Decimal::ZERO);
        let fill_price = fill.price.unwrap_or(Decimal::ZERO);
        let signed_fill_qty = match fill.side {
            OrderSide::Buy => fill.qty,
            OrderSide::Sell => -fill.qty,
        };
        let after_size = before_size + signed_fill_qty;

        if before_size == Decimal::ZERO {
            return FillMetrics {
                fee,
                realized_pnl: Decimal::ZERO,
                position_after: after_size,
                entry_price_after: fill_price,
                closed_qty: Decimal::ZERO,
                opening_fill: true,
            };
        }

        let reducing = before_size.signum() != signed_fill_qty.signum();
        let closed_qty = if reducing {
            before_size.abs().min(signed_fill_qty.abs())
        } else {
            Decimal::ZERO
        };
        let realized_pnl = if before_size > Decimal::ZERO {
            (fill_price - before_entry) * closed_qty
        } else {
            (before_entry - fill_price) * closed_qty
        };

        if !reducing {
            let total_abs = before_size.abs() + signed_fill_qty.abs();
            let entry_price_after = if total_abs == Decimal::ZERO {
                Decimal::ZERO
            } else {
                ((before_entry * before_size.abs()) + (fill_price * signed_fill_qty.abs())) / total_abs
            };
            return FillMetrics {
                fee,
                realized_pnl: Decimal::ZERO,
                position_after: after_size,
                entry_price_after,
                closed_qty: Decimal::ZERO,
                opening_fill: true,
            };
        }

        if after_size == Decimal::ZERO {
            return FillMetrics {
                fee,
                realized_pnl,
                position_after: Decimal::ZERO,
                entry_price_after: Decimal::ZERO,
                closed_qty,
                opening_fill: false,
            };
        }

        if after_size.signum() == before_size.signum() {
            return FillMetrics {
                fee,
                realized_pnl,
                position_after: after_size,
                entry_price_after: before_entry,
                closed_qty,
                opening_fill: false,
            };
        }

        FillMetrics {
            fee,
            realized_pnl,
            position_after: after_size,
            entry_price_after: fill_price,
            closed_qty,
            opening_fill: false,
        }
    }
}

fn key_from_intent(intent: &OrderIntent) -> String {
    format!(
        "{:?}:{}:{}:{}:{}",
        intent.side,
        intent.price.normalize(),
        intent.qty.normalize(),
        intent.reduce_only,
        intent.post_only
    )
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::{domain::OrderType, execution::ExistingOrder};

    fn intent(side: OrderSide, price: Decimal, qty: Decimal, reduce_only: bool) -> OrderIntent {
        OrderIntent {
            client_order_id: format!("{:?}-{}", side, price),
            symbol: "ETH_USDC_PERP".into(),
            side,
            order_type: OrderType::Limit,
            price,
            qty,
            reduce_only,
            post_only: true,
        }
    }

    fn existing(order_id: &str, intent: OrderIntent) -> ExistingOrder {
        ExistingOrder {
            order_id: order_id.into(),
            client_order_id: intent.client_order_id.clone(),
            intent,
        }
    }

    #[test]
    fn infers_reconciliation_fallback_fill_from_position_delta() {
        let engine = ReconcileEngine::new("ETH_USDC_PERP", dec!(0.000000001));
        let previous = ReconcileSnapshot {
            position: Some(Position {
                symbol: "ETH_USDC_PERP".into(),
                size: dec!(-0.003),
                entry_price: dec!(2255.45),
                unrealized_pnl: Decimal::ZERO,
            }),
            orders: vec![existing("37665702163", intent(OrderSide::Sell, dec!(2268.32), dec!(0.003), false))],
        };
        let current = ReconcileSnapshot {
            position: Some(Position {
                symbol: "ETH_USDC_PERP".into(),
                size: dec!(-0.006),
                entry_price: dec!(2255.45),
                unrealized_pnl: Decimal::ZERO,
            }),
            orders: vec![],
        };

        let fill = engine
            .infer_fill_from_reconciliation(previous.position.as_ref(), current.position.as_ref(), &previous.orders, &current.orders)
            .unwrap();
        assert_eq!(fill.side, OrderSide::Sell);
        assert_eq!(fill.qty, dec!(0.003));
        assert_eq!(fill.order_id.as_deref(), Some("37665702163"));
    }

    #[test]
    fn estimates_fill_metrics_for_short_reduction() {
        let engine = ReconcileEngine::new("ETH_USDC_PERP", dec!(0.000000001));
        let before = Position {
            symbol: "ETH_USDC_PERP".into(),
            size: dec!(-0.006),
            entry_price: dec!(2262.38),
            unrealized_pnl: Decimal::ZERO,
        };
        let fill = SyntheticFill {
            source: FillSource::ReconciliationFallback,
            symbol: "ETH_USDC_PERP".into(),
            side: OrderSide::Buy,
            qty: dec!(0.003),
            price: Some(dec!(2252.48)),
            order_id: Some("1".into()),
            client_order_id: Some("grid-buy-1".into()),
            fee: Decimal::ZERO,
        };
        let metrics = engine.estimate_fill_metrics(Some(&before), &fill);
        assert_eq!(metrics.position_after, dec!(-0.003));
        assert_eq!(metrics.entry_price_after, dec!(2262.38));
        assert_eq!(metrics.closed_qty, dec!(0.003));
        assert_eq!(metrics.realized_pnl, dec!(0.0297));
        assert!(!metrics.opening_fill);
    }
}
