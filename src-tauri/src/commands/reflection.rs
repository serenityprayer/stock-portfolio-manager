//! Tauri commands for P2 — round trips, time capsules, discipline score, memos.

use crate::db::Database;
use crate::models::reflection::*;
use crate::services::reflection_service;
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub fn get_round_trips(
    db: State<Database>,
    symbol: Option<String>,
) -> Result<Vec<RoundTrip>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    reflection_service::list_round_trips(&conn, symbol.as_deref())
}

#[tauri::command(rename_all = "camelCase")]
pub fn get_round_trip_stats(
    db: State<Database>,
    symbol: Option<String>,
) -> Result<RoundTripStats, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    reflection_service::round_trip_summary(&conn, symbol.as_deref())
}

#[tauri::command(rename_all = "camelCase")]
pub fn list_due_reviews(
    db: State<Database>,
    include_completed: Option<bool>,
) -> Result<Vec<DueReview>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    reflection_service::list_due_reviews(&conn, include_completed.unwrap_or(false))
}

/// 补齐历史交易的时光机。历史交易的到期日已过，会立即进入待复盘队列。
#[tauri::command(rename_all = "camelCase")]
pub fn backfill_time_capsules(db: State<Database>) -> Result<usize, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    reflection_service::backfill_time_capsules(&conn)
}

#[tauri::command(rename_all = "camelCase")]
pub fn complete_review(
    db: State<Database>,
    input: ReviewCompleteInput,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    reflection_service::complete_review(&conn, &input)
}

#[tauri::command(rename_all = "camelCase")]
pub fn get_discipline_score(db: State<Database>) -> Result<DisciplineScore, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    reflection_service::discipline_score(&conn)
}

#[tauri::command(rename_all = "camelCase")]
pub fn list_memos(db: State<Database>) -> Result<Vec<InvestmentMemo>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    reflection_service::list_memos(&conn)
}

#[tauri::command(rename_all = "camelCase")]
pub fn create_memo(db: State<Database>, input: MemoInput) -> Result<InvestmentMemo, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    reflection_service::create_memo(&conn, &input)
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_memo(db: State<Database>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    reflection_service::delete_memo(&conn, &id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn compare_memo(db: State<Database>, id: String) -> Result<MemoComparison, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    reflection_service::compare_memo(&conn, &id)
}
