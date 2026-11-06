//! Tauri commands for P1 watchlist / cooling-off / pre-trade check.

use crate::db::Database;
use crate::models::risk::{DisciplineStats, TradeVerdict, WatchItem, WatchItemInput};
use crate::services::discipline_service;
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub fn get_watchlist(db: State<Database>) -> Result<Vec<WatchItem>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    discipline_service::list_watchlist(&conn)
}

#[tauri::command(rename_all = "camelCase")]
pub fn add_watchlist_item(
    db: State<Database>,
    input: WatchItemInput,
) -> Result<WatchItem, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    discipline_service::add_watch_item(&conn, &input)
}

#[tauri::command(rename_all = "camelCase")]
pub fn remove_watchlist_item(db: State<Database>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    discipline_service::remove_watch_item(&conn, &id)
}

/// Front-end gate: show the verdict before the user fills in the whole form.
/// The backend re-evaluates independently inside `create_transaction`, so this
/// call cannot be skipped to bypass a BLOCK.
#[tauri::command(rename_all = "camelCase")]
pub fn precheck_trade(
    db: State<Database>,
    symbol: String,
    market: String,
    action: String,
) -> Result<TradeVerdict, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    discipline_service::evaluate_trade_intent(&conn, &symbol, &market, &action)
}

#[tauri::command(rename_all = "camelCase")]
pub fn get_discipline_stats(db: State<Database>) -> Result<DisciplineStats, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    discipline_service::discipline_stats(&conn)
}
