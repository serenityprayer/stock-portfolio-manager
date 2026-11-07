//! P4 — data quality, what-if stress test, contract liquidation distance.

use crate::models::risk::RiskFinding;
use serde::{Deserialize, Serialize};

/// FX shock in percent, applied to the base-currency translation of
/// non-cash positions. Cash holdings are deliberately excluded because the
/// aggregated `cash_value_base` does not retain per-account currencies.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FxShock {
    pub cny_percent: f64,
    pub hkd_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub label: String,
    pub global_percent: f64,
    pub stressed_total_assets: f64,
    pub stressed_drawdown: f64,
    /// normal | warning | blocked
    pub status: String,
    pub finding_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StressOverride {
    pub symbol: String,
    pub price_change_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StressDelta {
    pub symbol: String,
    pub old_price: f64,
    pub new_price: f64,
    pub old_value: f64,
    pub new_value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StressResult {
    pub as_of: String,
    pub baseline_total_assets: f64,
    pub stressed_total_assets: f64,
    pub baseline_drawdown: f64,
    pub stressed_drawdown: f64,
    /// normal | warning | blocked
    pub status: String,
    pub findings: Vec<RiskFinding>,
    pub deltas: Vec<StressDelta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractPriceInput {
    pub symbol: String,
    pub price: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractRiskItem {
    pub id: String,
    pub symbol: String,
    pub position_type: String,
    pub leverage: f64,
    pub open_price: f64,
    pub current_price: f64,
    pub liquidation_price: f64,
    pub distance_percent: f64,
    pub severity: String,
    pub message: String,
    pub has_quote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityIssue {
    /// price_range | price_jump | duplicate_symbol | quote_stale | rate_range | nav_jump
    pub kind: String,
    pub symbol: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataQualityReport {
    pub checked: i64,
    pub rejected: i64,
    pub issues: Vec<QualityIssue>,
}
