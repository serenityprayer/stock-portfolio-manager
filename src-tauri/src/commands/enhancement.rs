//! P4 — stress test, contract liquidation distance, data-quality health scan.

use crate::db::Database;
use crate::models::enhancement::*;
use crate::services::{contract_risk_service, data_quality, stress_service};
use rusqlite::params;
use tauri::State;

/// What-if preview. Does not advance HWM or write any risk event.
#[tauri::command(rename_all = "camelCase")]
pub fn stress_test(
    db: State<Database>,
    overrides: Vec<StressOverride>,
) -> Result<StressResult, String> {
    stress_service::run(&db, &overrides)
}

/// Side-by-side comparison of several market-wide shocks. `overrides` may
/// contain per-symbol shocks; any global "*" entry is ignored in favour of
/// each scenario's own level.
#[tauri::command(rename_all = "camelCase")]
pub fn compare_stress_scenarios(
    db: State<Database>,
    shocks: Vec<f64>,
    overrides: Vec<StressOverride>,
    fx: Option<FxShock>,
) -> Result<Vec<ScenarioResult>, String> {
    stress_service::run_batch(&db, &shocks, &overrides, fx.as_ref())
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

/// Escape hatch for a legitimate >95% price jump (split / reverse split /
/// ticker change). `validate_price` compares against the previous cached price,
/// so a stale anchor would reject the new price forever. Deleting the row makes
/// the next quote refresh count as a first observation (range check still
/// applies) and also removes case-duplicated rows such as `TCOM` / `tcom`.
#[tauri::command(rename_all = "camelCase")]
pub fn clear_quote_cache(db: State<Database>, symbol: String) -> Result<usize, String> {
    let sym = symbol.trim().to_uppercase();
    if sym.is_empty() {
        return Err("标的代码不能为空".to_string());
    }
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let removed = conn
        .execute(
            "DELETE FROM cached_quotes WHERE UPPER(symbol) = UPPER(?1)",
            params![sym],
        )
        .map_err(|e| e.to_string())?;
    Ok(removed)
}
