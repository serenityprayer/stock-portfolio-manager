use rusqlite::{params, Connection, Result};
use crate::models::crypto_contract::CryptoContract;

pub fn list(conn: &Connection) -> Result<Vec<CryptoContract>> {
    let mut stmt = conn.prepare(
        "SELECT id, symbol, name, asset_type, position_type, open_price, shares,
                leverage, margin, fee, exchange, notes, created_at, updated_at
         FROM crypto_contract ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(CryptoContract::from_db(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
            row.get(8)?,
            row.get(9)?,
            row.get(10)?,
            row.get(11)?,
            row.get(12)?,
            row.get(13)?,
        ))
    })?;
    rows.collect()
}

pub fn get_by_id(conn: &Connection, id: &str) -> Result<Option<CryptoContract>> {
    let mut stmt = conn.prepare(
        "SELECT id, symbol, name, asset_type, position_type, open_price, shares,
                leverage, margin, fee, exchange, notes, created_at, updated_at
         FROM crypto_contract WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map(params![id], |row| {
        Ok(CryptoContract::from_db(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
            row.get(8)?,
            row.get(9)?,
            row.get(10)?,
            row.get(11)?,
            row.get(12)?,
            row.get(13)?,
        ))
    })?;
    Ok(rows.next().transpose()?)
}

pub fn create(
    conn: &Connection,
    id: &str,
    symbol: &str,
    name: Option<&str>,
    asset_type: &str,
    position_type: &str,
    open_price: f64,
    shares: f64,
    leverage: f64,
    fee: Option<f64>,
    exchange: Option<&str>,
    notes: Option<&str>,
    created_at: &str,
    updated_at: &str,
) -> Result<()> {
    let notional = open_price * shares;
    let margin = notional / leverage + fee.unwrap_or(0.0);
    conn.execute(
        "INSERT INTO crypto_contract
         (id, symbol, name, asset_type, position_type, open_price, shares,
          leverage, margin, fee, exchange, notes, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![id, symbol, name, asset_type, position_type, open_price, shares,
                leverage, margin, fee, exchange, notes, created_at, updated_at],
    )?;
    Ok(())
}

pub fn update(
    conn: &Connection,
    id: &str,
    symbol: &str,
    name: Option<&str>,
    asset_type: &str,
    position_type: &str,
    open_price: f64,
    shares: f64,
    leverage: f64,
    fee: Option<f64>,
    exchange: Option<&str>,
    notes: Option<&str>,
    updated_at: &str,
) -> Result<()> {
    let notional = open_price * shares;
    let margin = notional / leverage + fee.unwrap_or(0.0);
    conn.execute(
        "UPDATE crypto_contract SET
            symbol=?2, name=?3, asset_type=?4, position_type=?5, open_price=?6, shares=?7,
            leverage=?8, margin=?9, fee=?10, exchange=?11, notes=?12, updated_at=?13
         WHERE id=?1",
        params![id, symbol, name, asset_type, position_type, open_price, shares,
                leverage, margin, fee, exchange, notes, updated_at],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM crypto_contract WHERE id = ?1", params![id])?;
    Ok(())
}
