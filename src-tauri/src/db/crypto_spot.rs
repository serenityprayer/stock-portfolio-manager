use rusqlite::{params, Connection, Result};
use crate::models::crypto_spot::CryptoSpot;

pub fn list(conn: &Connection) -> Result<Vec<CryptoSpot>> {
    let mut stmt = conn.prepare(
        "SELECT id, symbol, name, buy_price, shares, fee, exchange, notes, created_at, updated_at
         FROM crypto_spot ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(CryptoSpot::from_db(
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
        ))
    })?;
    rows.collect()
}

pub fn get_by_id(conn: &Connection, id: &str) -> Result<Option<CryptoSpot>> {
    let mut stmt = conn.prepare(
        "SELECT id, symbol, name, buy_price, shares, fee, exchange, notes, created_at, updated_at
         FROM crypto_spot WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map(params![id], |row| {
        Ok(CryptoSpot::from_db(
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
        ))
    })?;
    Ok(rows.next().transpose()?)
}

pub fn create(
    conn: &Connection,
    id: &str,
    symbol: &str,
    name: Option<&str>,
    buy_price: f64,
    shares: f64,
    fee: Option<f64>,
    exchange: Option<&str>,
    notes: Option<&str>,
    created_at: &str,
    updated_at: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO crypto_spot (id, symbol, name, buy_price, shares, fee, exchange, notes, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![id, symbol, name, buy_price, shares, fee, exchange, notes, created_at, updated_at],
    )?;
    Ok(())
}

pub fn update(
    conn: &Connection,
    id: &str,
    symbol: &str,
    name: Option<&str>,
    buy_price: f64,
    shares: f64,
    fee: Option<f64>,
    exchange: Option<&str>,
    notes: Option<&str>,
    updated_at: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE crypto_spot SET symbol=?2, name=?3, buy_price=?4, shares=?5, fee=?6, exchange=?7, notes=?8, updated_at=?9 WHERE id=?1",
        params![id, symbol, name, buy_price, shares, fee, exchange, notes, updated_at],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM crypto_spot WHERE id = ?1", params![id])?;
    Ok(())
}
