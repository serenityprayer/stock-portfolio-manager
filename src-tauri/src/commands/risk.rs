use crate::db::Database;
use crate::models::risk::{
    HwmState, RiskDashboard, RiskFinding, RiskRuleConfig,
};
use crate::services::risk_service;
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub fn get_risk_dashboard(db: State<Database>) -> Result<RiskDashboard, String> {
    risk_service::get_dashboard(&db)
}

/// 手动全量体检：重算规则、落库事件并返回命中项。
#[tauri::command(rename_all = "camelCase")]
pub fn evaluate_risk(db: State<Database>) -> Result<Vec<RiskFinding>, String> {
    risk_service::evaluate_risk(&db)
}

#[tauri::command(rename_all = "camelCase")]
pub fn acknowledge_risk_event(db: State<Database>, id: String) -> Result<(), String> {
    risk_service::acknowledge_event(&db, &id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn get_risk_rules(db: State<Database>) -> Result<Vec<RiskRuleConfig>, String> {
    risk_service::list_rules(&db)
}

#[tauri::command(rename_all = "camelCase")]
pub fn update_risk_rule(
    db: State<Database>,
    id: String,
    enabled: Option<bool>,
    threshold: Option<f64>,
) -> Result<(), String> {
    risk_service::update_rule(&db, &id, enabled, threshold)
}

#[tauri::command(rename_all = "camelCase")]
pub fn get_hwm_state(db: State<Database>) -> Result<HwmState, String> {
    risk_service::get_hwm(&db)
}

/// 校准需填原因，且会写一条已解决的审计事件。
#[tauri::command(rename_all = "camelCase")]
pub fn recalibrate_hwm(db: State<Database>, reason: String) -> Result<HwmState, String> {
    risk_service::recalibrate_hwm(&db, &reason)
}
