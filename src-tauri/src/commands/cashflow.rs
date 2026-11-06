use crate::db::Database;
use crate::models::risk::ExternalCashFlow;
use crate::services::cashflow_service;
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub fn list_external_flows(db: State<Database>) -> Result<Vec<ExternalCashFlow>, String> {
    cashflow_service::list_external_flows(&db)
}

/// amount 恒为正数；方向由 flowType 决定（DEPOSIT 入金 / WITHDRAW 出金 /
/// DIVIDEND_ADJUST 资产校准）。记录与现金持仓调整在同一事务内完成。
#[tauri::command(rename_all = "camelCase")]
pub fn record_external_flow(
    db: State<Database>,
    account_id: String,
    currency: String,
    flow_type: String,
    amount: f64,
    occurred_at: String,
    note: Option<String>,
) -> Result<ExternalCashFlow, String> {
    cashflow_service::record_external_flow(
        &db,
        &account_id,
        &currency,
        &flow_type,
        amount,
        &occurred_at,
        note,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_external_flow(db: State<Database>, id: String) -> Result<(), String> {
    cashflow_service::delete_external_flow(&db, &id)
}
