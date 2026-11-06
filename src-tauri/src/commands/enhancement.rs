//! P4 — stress test, contract liquidation distance, data-quality health scan.

use crate::db::Database;
use crate::models::enhancement::*;
use crate::services::{contract_risk_service, data_quality, stress_service};
use tauri::State;

/// What-if preview. Does not advance HWM or write any risk event.
#[tauri::command(rename_all = "camelCase")]
pub fn stress_test(
    db: State<Database>,
    overrides: Vec<StressOverride>,
) -> Result<StressResult, String> {
    stress_service::run(&db, &overrides)
}

/// `prices` can be empty: the service falls back to cached quotes, then to the
/// open price (clearly flagged as an estimate).
#[tauri::command(rename_all = "camelCase")]
pub fn evaluate_contract_risk(
    db: State<Database>,
    prices: Vec<ContractPriceInput>,
) -> Result<Vec<ContractRiskItem>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    contract_risk_service::evaluate(&conn, &prices)
}

#[tauri::command(rename_all = "camelCase")]
pub fn check_data_quality(db: State<Database>) -> Result<DataQualityReport, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    data_quality::check(&conn)
}
