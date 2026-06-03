use crate::db::Database;
use crate::models::crypto_spot::CryptoSpot;
use tauri::State;
use uuid::Uuid;
use chrono::Utc;

#[tauri::command(rename_all = "camelCase")]
pub fn list_crypto_spots(db: State<Database>) -> Result<Vec<CryptoSpot>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    crate::db::crypto_spot::list(&conn).map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn get_crypto_spot_by_id(
    db: State<Database>,
    id: String,
) -> Result<Option<CryptoSpot>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    crate::db::crypto_spot::get_by_id(&conn, &id).map_err(|e| e.to_string())
}

// ---------- commands ----------

#[tauri::command(rename_all = "camelCase")]
pub fn create_crypto_spot(
    db: State<Database>,
    symbol: String,
    name: Option<String>,
    buy_price: f64,
    shares: f64,
    fee: Option<f64>,
    exchange: Option<String>,
    notes: Option<String>,
) -> Result<CryptoSpot, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();

    crate::db::crypto_spot::create(
        &conn,
        &id,
        &symbol,
        name.as_deref(),
        buy_price,
        shares,
        fee,
        exchange.as_deref(),
        notes.as_deref(),
        &now,
        &now,
    )
    .map_err(|e| e.to_string())?;

    Ok(CryptoSpot {
        id,
        symbol,
        name,
        buy_price,
        shares,
        fee,
        exchange,
        notes,
        current_price: None,
        market_value: None,
        pnl: None,
        pnl_pct: None,
        created_at: now.clone(),
        updated_at: now,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn update_crypto_spot(
    db: State<Database>,
    id: String,
    symbol: Option<String>,
    name: Option<String>,
    buy_price: Option<f64>,
    shares: Option<f64>,
    fee: Option<f64>,
    exchange: Option<String>,
    notes: Option<String>,
) -> Result<CryptoSpot, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let now = Utc::now().to_rfc3339();

    let existing = crate::db::crypto_spot::get_by_id(&conn, &id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Crypto spot not found".to_string())?;

    let symbol = symbol.unwrap_or(existing.symbol);
    let name = name.or(existing.name);
    let buy_price = buy_price.unwrap_or(existing.buy_price);
    let shares = shares.unwrap_or(existing.shares);
    let fee = fee.or(existing.fee);
    let exchange = exchange.or(existing.exchange);
    let notes = notes.or(existing.notes);

    crate::db::crypto_spot::update(
        &conn,
        &id,
        &symbol,
        name.as_deref(),
        buy_price,
        shares,
        fee,
        exchange.as_deref(),
        notes.as_deref(),
        &now,
    )
    .map_err(|e| e.to_string())?;

    Ok(CryptoSpot {
        id,
        symbol,
        name,
        buy_price,
        shares,
        fee,
        exchange,
        notes,
        current_price: None,
        market_value: None,
        pnl: None,
        pnl_pct: None,
        created_at: existing.created_at,
        updated_at: now,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_crypto_spot(
    db: State<Database>,
    id: String,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    crate::db::crypto_spot::delete(&conn, &id).map_err(|e| e.to_string())
}
