use crate::{execution::ExistingOrder, reconcile::SyntheticFill, Position};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RuntimeState {
    pub working_orders: Vec<ExistingOrder>,
    pub recent_fills: Vec<SyntheticFill>,
    pub position: Option<Position>,
    pub last_mid_price: Option<rust_decimal::Decimal>,
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryStateStore {
    state: RuntimeState,
}

impl InMemoryStateStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self) -> RuntimeState {
        self.state.clone()
    }

    pub fn restore(&mut self, state: RuntimeState) {
        self.state = state;
    }

    pub fn set_working_orders(&mut self, orders: Vec<ExistingOrder>) {
        self.state.working_orders = orders;
    }

    pub fn set_position(&mut self, position: Position) {
        self.state.position = Some(position);
    }

    pub fn set_last_mid_price(&mut self, mid_price: rust_decimal::Decimal) {
        self.state.last_mid_price = Some(mid_price);
    }

    pub fn push_fill(&mut self, fill: SyntheticFill, limit: usize) {
        let limit = limit.max(1);
        self.state.recent_fills.insert(0, fill);
        self.state.recent_fills.truncate(limit);
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::{FillSource, OrderSide};

    #[test]
    fn state_store_returns_copies() {
        let mut store = InMemoryStateStore::new();
        store.set_position(Position {
            symbol: "ETH_USDC_PERP".into(),
            size: dec!(-0.003),
            entry_price: dec!(2255.45),
            unrealized_pnl: dec!(0),
        });
        let mut snapshot = store.get();
        snapshot.position.as_mut().unwrap().size = dec!(0);
        assert_eq!(store.get().position.unwrap().size, dec!(-0.003));
    }

    #[test]
    fn state_store_limits_recent_fills() {
        let mut store = InMemoryStateStore::new();
        for idx in 0..3 {
            store.push_fill(
                SyntheticFill {
                    source: FillSource::ReconciliationFallback,
                    symbol: "ETH_USDC_PERP".into(),
                    side: OrderSide::Buy,
                    qty: dec!(0.003),
                    price: Some(dec!(2252.48)),
                    order_id: Some(idx.to_string()),
                    client_order_id: Some(format!("grid-buy-{idx}")),
                    fee: dec!(0),
                },
                2,
            );
        }
        assert_eq!(store.get().recent_fills.len(), 2);
        assert_eq!(store.get().recent_fills[0].order_id.as_deref(), Some("2"));
    }
}
