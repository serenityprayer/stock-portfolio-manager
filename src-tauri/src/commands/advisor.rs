//! P3 — dialectic advisor Tauri commands.

use crate::db::Database;
use crate::models::advisor::{AdvisorReply, TradeAnalysis};
use crate::services::advisor_service;
use tauri::State;

/// Challenge a user assumption. Returns local evidence only when no API key is
/// configured (degraded mode) instead of failing the whole feature.
#[tauri::command(rename_all = "camelCase")]
pub async fn chat_with_advisor(
    db: State<'_, Database>,
    message: String,
) -> Result<AdvisorReply, String> {
    advisor_service::chat_with_advisor(db.inner(), &message).await
}

/// Analyse a single transaction's original thesis and leave an auditable
/// `ai_analysis` review row.
#[tauri::command(rename_all = "camelCase")]
pub async fn analyze_trade(
    db: State<'_, Database>,
    transaction_id: String,
) -> Result<TradeAnalysis, String> {
    advisor_service::analyze_trade(db.inner(), &transaction_id).await
}
