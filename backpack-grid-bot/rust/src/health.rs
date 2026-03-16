use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Starting,
    Active,
    Degraded,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    Manual,
    PriceOutOfRange,
    KillSwitch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthIssue {
    SyntheticFillObserved,
    OpenOrderMismatch,
    PositionMissing,
    MarkPriceMissing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeHealth {
    pub service_state: ServiceState,
    pub pause_reason: Option<PauseReason>,
    pub consecutive_errors: u32,
    pub last_cycle_unix_ms: Option<i64>,
    pub last_success_unix_ms: Option<i64>,
    pub last_reconciliation_unix_ms: Option<i64>,
    pub last_mid_price: Option<Decimal>,
    pub issues: Vec<HealthIssue>,
}

impl Default for RuntimeHealth {
    fn default() -> Self {
        Self {
            service_state: ServiceState::Starting,
            pause_reason: None,
            consecutive_errors: 0,
            last_cycle_unix_ms: None,
            last_success_unix_ms: None,
            last_reconciliation_unix_ms: None,
            last_mid_price: None,
            issues: vec![],
        }
    }
}

impl RuntimeHealth {
    pub fn mark_success(&mut self, service_state: ServiceState, issues: Vec<HealthIssue>, last_mid_price: Option<Decimal>) {
        let now = now_unix_ms();
        self.service_state = service_state;
        self.consecutive_errors = 0;
        self.last_cycle_unix_ms = Some(now);
        self.last_success_unix_ms = Some(now);
        self.last_reconciliation_unix_ms = Some(now);
        self.last_mid_price = last_mid_price;
        self.issues = issues;
    }

    pub fn mark_error(&mut self) {
        self.consecutive_errors = self.consecutive_errors.saturating_add(1);
        self.last_cycle_unix_ms = Some(now_unix_ms());
        self.service_state = ServiceState::Degraded;
    }

    pub fn pause(&mut self, reason: PauseReason, last_mid_price: Option<Decimal>) {
        let now = now_unix_ms();
        self.service_state = ServiceState::Paused;
        self.pause_reason = Some(reason);
        self.last_cycle_unix_ms = Some(now);
        self.last_mid_price = last_mid_price;
    }
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    #[test]
    fn health_marks_success_and_error() {
        let mut health = RuntimeHealth::default();
        health.mark_error();
        assert_eq!(health.service_state, ServiceState::Degraded);
        assert_eq!(health.consecutive_errors, 1);

        health.mark_success(ServiceState::Active, vec![HealthIssue::SyntheticFillObserved], Some(dec!(2260.4)));
        assert_eq!(health.service_state, ServiceState::Active);
        assert_eq!(health.consecutive_errors, 0);
        assert_eq!(health.last_mid_price, Some(dec!(2260.4)));
        assert_eq!(health.issues, vec![HealthIssue::SyntheticFillObserved]);
    }
}
