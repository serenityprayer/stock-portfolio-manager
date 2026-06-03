use crate::db::Database;
use crate::models::crypto_contract::CryptoContract;
use tauri::State;
use uuid::Uuid;
use chrono::Utc;

#[tauri::command(rename_all = "camelCase")]
pub fn list_crypto_contracts(db: State<Database>) -> Result<Vec<CryptoContract>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    crate::db::crypto_contract::list(&conn).map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn get_crypto_contract_by_id(
    db: State<Database>,
    id: String,
) -> Result<Option<CryptoContract>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    crate::db::crypto_contract::get_by_id(&conn, &id).map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn create_crypto_contract(
    db: State<Database>,
    symbol: String,
    name: Option<String>,
    asset_type: String,
    position_type: String,
    open_price: f64,
    shares: f64,
    leverage: f64,
    fee: Option<f64>,
    exchange: Option<String>,
    notes: Option<String>,
) -> Result<CryptoContract, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();

    let asset_type = if asset_type.is_empty() {
        "crypto".to_string()
    } else {
        asset_type
    };

    crate::db::crypto_contract::create(
        &conn,
        &id,
        &symbol,
        name.as_deref(),
        &asset_type,
        &position_type,
        open_price,
        shares,
        leverage,
        fee,
        exchange.as_deref(),
        notes.as_deref(),
        &now,
        &now,
    )
    .map_err(|e| e.to_string())?;

    // 强平价格仅 crypto 类型计算
    let liquidation_price = if leverage > 0.0 && asset_type == "crypto" {
        let maintenance_margin_rate = 0.005;
        match position_type.as_str() {
            "long" => Some(open_price * (1.0 - (1.0 / leverage) + maintenance_margin_rate)),
            "short" => Some(open_price * (1.0 + (1.0 / leverage) - maintenance_margin_rate)),
            _ => None,
        }
    } else {
        None
    };

    Ok(CryptoContract {
        id,
        symbol,
        name,
        asset_type,
        position_type,
        open_price,
        shares,
        leverage,
        margin: open_price * shares / leverage + fee.unwrap_or(0.0),
        fee,
        exchange,
        notes,
        current_price: None,
        market_value: None,
        pnl: None,
        pnl_pct: None,
        liquidation_price,
        created_at: now.clone(),
        updated_at: now,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn update_crypto_contract(
    db: State<Database>,
    id: String,
    symbol: String,
    name: Option<String>,
    asset_type: String,
    position_type: String,
    open_price: f64,
    shares: f64,
    leverage: f64,
    fee: Option<f64>,
    exchange: Option<String>,
    notes: Option<String>,
) -> Result<CryptoContract, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let now = Utc::now().to_rfc3339();

    let existing = crate::db::crypto_contract::get_by_id(&conn, &id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Crypto contract not found".to_string())?;

    let symbol = if symbol.is_empty() {
        existing.symbol
    } else {
        symbol
    };
    let asset_type = if asset_type.is_empty() {
        existing.asset_type
    } else {
        asset_type
    };
    let position_type = if position_type.is_empty() {
        existing.position_type
    } else {
        position_type
    };

    crate::db::crypto_contract::update(
        &conn,
        &id,
        &symbol,
        name.as_deref(),
        &asset_type,
        &position_type,
        open_price,
        shares,
        leverage,
        fee,
        exchange.as_deref(),
        notes.as_deref(),
        &now,
    )
    .map_err(|e| e.to_string())?;

    let liquidation_price = if leverage > 0.0 && asset_type == "crypto" {
        let maintenance_margin_rate = 0.005;
        match position_type.as_str() {
            "long" => Some(open_price * (1.0 - (1.0 / leverage) + maintenance_margin_rate)),
            "short" => Some(open_price * (1.0 + (1.0 / leverage) - maintenance_margin_rate)),
            _ => None,
        }
    } else {
        None
    };

    Ok(CryptoContract {
        id,
        symbol,
        name,
        asset_type,
        position_type,
        open_price,
        shares,
        leverage,
        margin: open_price * shares / leverage + fee.unwrap_or(0.0),
        fee,
        exchange,
        notes,
        current_price: None,
        market_value: None,
        pnl: None,
        pnl_pct: None,
        liquidation_price,
        created_at: existing.created_at,
        updated_at: now,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_crypto_contract(
    db: State<Database>,
    id: String,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    crate::db::crypto_contract::delete(&conn, &id).map_err(|e| e.to_string())
}
