//! Data contracts for the Risk & Discipline Engine.
//!
//! Field naming deliberately mirrors the rest of the project (snake_case in
//! Rust, serialised as-is). Frontend types in `src/types/index.ts` match these
//! one-to-one.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const SEV_INFO: &str = "info";
pub const SEV_WARNING: &str = "warning";
pub const SEV_CRITICAL: &str = "critical";
pub const SEV_BLOCK: &str = "block";

pub const STATUS_ACKNOWLEDGED: &str = "acknowledged";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskRuleConfig {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub severity: String,
    pub enabled: bool,
    pub threshold: Option<f64>,
    pub params: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskEvent {
    pub id: String,
    pub rule_id: String,
    pub symbol: Option<String>,
    pub account_id: Option<String>,
    pub severity: String,
    pub status: String,
    pub metric_value: Option<f64>,
    pub threshold: Option<f64>,
    pub message: String,
    pub context: Option<String>,
    pub first_triggered_at: String,
    pub last_triggered_at: String,
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HwmState {
    pub scope_key: String,
    pub peak_unit_nav: f64,
    pub peak_date: String,
    pub current_unit_nav: f64,
    /// 组合总资产（折算 base currency）在净值序列锚定时刻的值。
    pub base_value: f64,
    pub base_date: String,
    /// 自锚定日起累计净外部现金流（入金为正、出金为负）。
    pub net_external_flow: f64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalCashFlow {
    pub id: String,
    pub account_id: String,
    pub currency: String,
    pub market: String,
    pub flow_type: String,
    /// 规范化后的带符号金额：入金为正、出金为负。
    pub amount: f64,
    pub occurred_at: String,
    pub note: Option<String>,
    pub created_at: String,
}

/// A single rule hit produced by the pure evaluation engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFinding {
    pub rule_id: String,
    pub severity: String,
    pub symbol: Option<String>,
    pub metric: f64,
    pub threshold: f64,
    pub message: String,
    pub context: Value,
}

/// Real-time position truth, built from `holdings` + quotes.
/// Never derived from daily/quarterly snapshots (see design doc §5.5).
#[derive(Debug, Clone)]
pub struct RiskPosition {
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub account_id: String,
    pub currency: String,
    pub shares: f64,
    pub avg_cost: f64,
    pub price: f64,
    pub market_value_base: f64,
    pub pnl_percent: f64,
    pub has_quote: bool,
    pub quote_age_secs: i64,
    pub is_cash: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteFreshness {
    pub has_quotes: bool,
    pub stale_symbols: Vec<String>,
    pub missing_symbols: Vec<String>,
    pub oldest_quote_age_secs: i64,
    pub stale_threshold_secs: i64,
}

#[derive(Debug, Clone)]
pub struct RiskContext {
    pub as_of: String,
    pub base_currency: String,
    /// 非现金、shares > 0 的持仓。
    pub positions: Vec<RiskPosition>,
    /// 现金持仓（可为负，代表融资/透支），折算到 base currency。
    pub cash_value_base: f64,
    /// 总资产 = 持仓市值 + 现金。买卖只是内部形态转换，不改变该值。
    pub total_assets_base: f64,
    pub unit_nav: f64,
    pub peak_unit_nav: f64,
    pub rules: HashMap<String, RiskRuleConfig>,
    pub freshness: QuoteFreshness,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskDashboard {
    /// normal | warning | blocked | degraded (quotes missing/stale)
    pub status: String,
    /// True when at least one position lacked a usable quote, so the three
    /// iron laws did not fully run. Surfaced as a grey badge in the UI.
    pub degraded: bool,
    pub as_of: String,
    pub base_currency: String,
    pub total_assets: f64,
    /// 现金持仓折算额（可为负＝融资/透支）。
    pub cash_value: f64,
    pub unit_nav: f64,
    pub peak_unit_nav: f64,
    /// 当前回撤百分比（负值代表回撤）。
    pub current_drawdown: f64,
    pub drawdown_threshold: f64,
    pub stop_loss_count: usize,
    pub stop_loss_threshold: f64,
    pub max_weight: f64,
    pub max_weight_symbol: String,
    pub concentration_threshold: f64,
    pub net_external_flow: f64,
    pub hwm_base_date: String,
    /// 口径说明：首版 HWM 自启用日起算，历史快照不参与熔断。
    pub hwm_note: String,
    pub position_count: usize,
    pub quote_freshness: QuoteFreshness,
    pub findings: Vec<RiskFinding>,
    pub events: Vec<RiskEvent>,
    pub rules: Vec<RiskRuleConfig>,
}

// ────────────────────────────────────────────────────────────────
// P1: watchlist / cooling period / trade discipline
// ────────────────────────────────────────────────────────────────

pub const VERDICT_PASS: &str = "PASS";
pub const VERDICT_WARN: &str = "WARN";
pub const VERDICT_BLOCK: &str = "BLOCK";

pub const RULE_COOLING_PERIOD: &str = "COOLING_PERIOD";
pub const RULE_UNPLANNED_TRADE: &str = "UNPLANNED_TRADE";

/// One entry in the 7-day cooling-off watchlist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchItem {
    pub id: String,
    pub symbol: String,
    pub name: Option<String>,
    pub market: String,
    pub currency: Option<String>,
    pub added_at: String,
    pub thesis: Option<String>,
    pub target_price: Option<f64>,
    pub status: String,
    pub promoted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Days since `added_at` (fractional, UTC based).
    pub cooling_days: f64,
    /// Days still required before the item becomes eligible.
    pub days_remaining: f64,
    pub eligible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchItemInput {
    pub symbol: String,
    pub name: Option<String>,
    pub market: String,
    pub currency: Option<String>,
    pub thesis: Option<String>,
    pub target_price: Option<f64>,
    /// Optional back-date in YYYY-MM-DD form; defaults to now.
    pub added_at: Option<String>,
}

/// Result of the pre-trade discipline check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeVerdict {
    /// PASS | WARN | BLOCK
    pub level: String,
    pub message: String,
    pub symbol: String,
    pub action: String,
    pub has_position: bool,
    pub in_watchlist: bool,
    pub cooling_days: Option<f64>,
    pub days_remaining: Option<f64>,
    pub is_unplanned: bool,
    /// BLOCK requires a non-empty override reason before the trade can proceed.
    pub requires_reason: bool,
    pub rule_ids: Vec<String>,
}

/// Optional payload on `create_transaction`. When absent the trade is treated
/// as a historical/backfill import and no discipline gate is applied — this is
/// what keeps the five CSV import dialogs working unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRiskContext {
    pub override_reason: Option<String>,
    pub ack_warning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisciplineStats {
    pub total: i64,
    pub blocked: i64,
    pub warn: i64,
    pub overridden: i64,
    pub unplanned: i64,
    /// Percentage of BUY decisions that were not overridden / unplanned.
    pub compliance_rate: f64,
}
