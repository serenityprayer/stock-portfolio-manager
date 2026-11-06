//! Data contracts for P2 — LIFO round trips, time capsules, discipline score
//! and investment memos. Field names mirror `src/types/index.ts`.

use serde::{Deserialize, Serialize};

pub const REVIEW_TIME_CAPSULE: &str = "time_capsule";
pub const REVIEW_AI_ANALYSIS: &str = "ai_analysis";
pub const QUALITY_CORRECT: &str = "correct";
pub const QUALITY_WRONG: &str = "wrong";
pub const QUALITY_PENDING: &str = "pending";

/// One completed LIFO round trip (a SELL matched against prior BUY/OPEN lots).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundTrip {
    pub symbol: String,
    pub market: String,
    pub open_at: String,
    pub close_at: String,
    pub holding_days: i64,
    pub qty: f64,
    pub avg_open_price: f64,
    pub avg_close_price: f64,
    /// Net of the proportional commissions on both legs.
    pub realized_pnl: f64,
    pub return_rate: f64,
    pub open_note: Option<String>,
    pub close_note: Option<String>,
    /// True when the SELL consumed more than one open lot.
    pub split_lots: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundTripStats {
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub win_rate: f64,
    /// Gross profit / absolute gross loss. `None` when there is no loss.
    pub profit_factor: Option<f64>,
    pub total_realized_pnl: f64,
    pub avg_holding_days: f64,
    pub best_trade: Option<RoundTrip>,
    pub worst_trade: Option<RoundTrip>,
    /// Open lots that were never closed (informational, not part of win rate).
    pub open_lot_count: usize,
}

/// A due (or completed) T+30 review, enriched with the original trade thesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DueReview {
    pub id: String,
    pub transaction_id: Option<String>,
    pub symbol: String,
    pub market: String,
    pub review_type: String,
    pub decision_quality: Option<String>,
    pub content: Option<String>,
    pub due_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub traded_at: Option<String>,
    /// 当初的买入理由（transactions.notes）。
    pub open_note: Option<String>,
    /// 破例理由非空的交易优先展示 —— 正好检验「那次破例值不值」。
    pub override_reason: Option<String>,
    pub is_priority: bool,
    pub days_overdue: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewCompleteInput {
    pub id: String,
    pub quality: String,
    pub content: String,
}

/// 纪律分：只作趋势展示，不设排行榜、不做惩罚（§8.3）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisciplineScore {
    pub score: f64,
    /// 0–100 分项，null 表示样本不足（不计入加权，按 100 处理并标注）。
    pub cooling_compliance: Option<f64>,
    pub planned_ratio: Option<f64>,
    pub stop_loss_execution: Option<f64>,
    pub review_completion: Option<f64>,
    pub cooling_detail: String,
    pub planned_detail: String,
    pub stop_loss_detail: String,
    pub review_detail: String,
    pub sample_size: i64,
    /// 样本太小时分数只作参考。
    pub low_sample: bool,
}

// ────────────────────────────────────────────────────────────────
// Investment memos + frozen portfolio snapshot
// ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotPosition {
    pub symbol: String,
    pub name: String,
    pub shares: f64,
    pub avg_cost: f64,
    pub price: f64,
    pub market_value_base: f64,
    pub weight: f64,
    pub has_quote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEvent {
    pub rule_id: String,
    pub symbol: Option<String>,
    pub severity: String,
    pub message: String,
}

/// Frozen at memo creation time. Never recomputed afterwards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoldingsSnapshot {
    pub as_of: String,
    pub base_currency: String,
    pub total_assets_base: f64,
    pub cash_value_base: f64,
    pub unit_nav: f64,
    pub peak_unit_nav: f64,
    pub positions: Vec<SnapshotPosition>,
    pub active_events: Vec<SnapshotEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoInput {
    pub title: String,
    pub content: String,
    pub tags: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestmentMemo {
    pub id: String,
    pub title: String,
    pub content: String,
    pub tags: Option<String>,
    pub holdings_snapshot: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 并排对比「当时怎么想」与「后来怎么样」。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoComparison {
    pub memo: InvestmentMemo,
    pub then_snapshot: Option<HoldingsSnapshot>,
    pub now_snapshot: HoldingsSnapshot,
    /// 快照中仍持有的标的，按当时→现在的市值变化。
    pub changes: Vec<MemoPositionChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoPositionChange {
    pub symbol: String,
    pub then_weight: f64,
    pub now_weight: f64,
    pub then_value: f64,
    pub now_value: f64,
    pub value_change: f64,
}
