use crate::db::Database;
use crate::models::crypto_contract::CryptoContract;
use rusqlite::Connection;
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

/// 平仓合约（支持部分/全部平仓）
#[tauri::command(rename_all = "camelCase")]
pub fn close_crypto_contract(
    db: State<Database>,
    id: String,
    close_price: f64,
    close_shares: f64,
    close_fee: Option<f64>,
    notes: Option<String>,
) -> Result<serde_json::Value, String> {
    let mut conn_guard = db.conn.lock().map_err(|e| e.to_string())?;
    let conn: &mut Connection = &mut conn_guard;
    let now = Utc::now().to_rfc3339();
    let fee = close_fee.unwrap_or(0.0);

    let (remaining, history_id) =
        crate::db::crypto_contract::close_contract(
            conn,
            &id,
            close_price,
            close_shares,
            fee,
            notes.as_deref(),
            &now,
        )
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "historyId": history_id,
        "remainingShares": remaining.as_ref().map(|c| c.shares),
        "fullyClosed": remaining.is_none(),
    }))
}

/// 查询成交历史（平仓 + 加仓）
#[tauri::command(rename_all = "camelCase")]
pub fn list_contract_history(
    db: State<Database>,
) -> Result<Vec<serde_json::Value>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    crate::db::crypto_contract::list_contract_history(&conn).map_err(|e| e.to_string())
}

/// 加仓：在现有合约上追加仓位，重新计算加权平均开仓价
#[tauri::command(rename_all = "camelCase")]
pub fn add_crypto_position(
    db: State<Database>,
    id: String,
    add_shares: f64,
    add_price: f64,
    add_fee: Option<f64>,
) -> Result<CryptoContract, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let now = Utc::now().to_rfc3339();
    let fee = add_fee.unwrap_or(0.0);
    crate::db::crypto_contract::add_position(
        &conn,
        &id,
        add_shares,
        add_price,
        fee,
        &now,
    )
    .map_err(|e| e.to_string())
}
