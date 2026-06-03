use chrono::Utc;
use rusqlite::{params, Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};
use uuid::Uuuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CryptoContract {
    pub id: String,
    pub symbol: String,
    pub name: Option<String>,
    #[serde(rename = "positionType")]
    pub position_type: String, // "long" or "short"
    #[serde(rename = "entryPrice")]
    pub entry_price: f64,
    pub shares: f64,
    pub leverage: i32,
    pub margin: f64,
    #[serde(rename = "liquidationPrice")]
    pub liquidation_price: Option<f64>,
    #[serde(rename = "takeProfit")]
    pub take_profit: Option<f64>,
    #[serde(rename = "stopLoss")]
    pub stop_loss: Option<f64>,
    pub exchange: Option<String>,
    pub notes: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    // Computed fields (not stored in DB)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_price: Option<f64>,
    #[serde(rename = "marketValue", skip_serializing_if = "Option::is_none")]
    pub market_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pnl: Option<f64>,
    #[serde(rename = "pnlPct", skip_serializing_if = "Option::is_none")]
    pub pnl_pct: Option<f64>,
}

pub fn create_table(conn: &Connection) -> SqlResult<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS crypto_contract (
            id TEXT PRIMARY KEY,
            symbol TEXT NOT NULL,
            name TEXT,
            position_type TEXT NOT NULL CHECK(position_type IN ('long', 'short')),
            entry_price REAL NOT NULL,
            shares REAL NOT NULL,
            leverage INTEGER NOT NULL DEFAULT 1,
            margin REAL NOT NULL,
            liquidation_price REAL,
            take_profit REAL,
            stop_loss REAL,
            exchange TEXT,
            notes TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
        [],
    )?;
    Ok(())
}

pub fn create(
    conn: &Connection,
    symbol: &str,
    name: Option<&str>,
    position_type: &str,
    entry_price: f64,
    shares: f64,
    leverage: i32,
    margin: f64,
    liquidation_price: Option<f64>,
    take_profit: Option<f64>,
    stop_loss: Option<f64>,
    exchange: Option<&str>,
    notes: Option<&str>,
) -> SqlResult<CryptoContract> {
    let id = Uuuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO crypto_contract (
            id, symbol, name, position_type, entry_price, shares, leverage,
            margin, liquidation_price, take_profit, stop_loss, exchange, notes,
            created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            &id,
            &symbol.to_uppercase(),
            name,
            &position_type.to_lowercase(),
            &entry_price,
            &shares,
            &leverage,
            &margin,
            &liquidation_price,
            &take_profit,
            &stop_loss,
            exchange,
            notes,
            &now,
            &now,
        ],
    )?;

    Ok(CryptoContract {
        id,
        symbol: symbol.to_uppercase(),
        name: name.map(|s| s.to_string()),
        position_type: position_type.to_lowercase(),
        entry_price,
        shares,
        leverage,
        margin,
        liquidation_price,
        take_profit,
        stop_loss,
        exchange: exchange.map(|s| s.to_string()),
        notes: notes.map(|s| s.to_string()),
        created_at: now.clone(),
        updated_at: now,
        current_price: None,
        market_value: None,
        pnl: None,
        pnl_pct: None,
    })
}

pub fn list(conn: &Connection) -> SqlResult<Vec<CryptoContract>> {
    let mut stmt = conn.prepare(
        "SELECT id, symbol, name, position_type, entry_price, shares, leverage,
                margin, liquidation_price, take_profit, stop_loss, exchange, notes,
                created_at, updated_at
         FROM crypto_contract ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(CryptoContract {
            id: row.get(0)?,
            symbol: row.get(1)?,
            name: row.get(2)?,
            position_type: row.get(3)?,
            entry_price: row.get(4)?,
            shares: row.get(5)?,
            leverage: row.get(6)?,
            margin: row.get(7)?,
            liquidation_price: row.get(8)?,
            take_profit: row.get(9)?,
            stop_loss: row.get(10)?,
            exchange: row.get(11)?,
            notes: row.get(12)?,
            created_at: row.get(13)?,
            updated_at: row.get(14)?,
            current_price: None,
            market_value: None,
            pnl: None,
            pnl_pct: None,
        })
    })?;
    rows.collect()
}

pub fn get(conn: &Connection, id: &str) -> SqlResult<Option<CryptoContract>> {
    let mut stmt = conn.prepare(
        "SELECT id, symbol, name, position_type, entry_price, shares, leverage,
                margin, liquidation_price, take_profit, stop_loss, exchange, notes,
                created_at, updated_at
         FROM crypto_contract WHERE id = ?",
    )?;
    stmt.query_row(params![id], |row| {
        Ok(CryptoContract {
            id: row.get(0)?,
            symbol: row.get(1)?,
            name: row.get(2)?,
            position_type: row.get(3)?,
            entry_price: row.get(4)?,
            shares: row.get(5)?,
            leverage: row.get(6)?,
            margin: row.get(7)?,
            liquidation_price: row.get(8)?,
            take_profit: row.get(9)?,
            stop_loss: row.get(10)?,
            exchange: row.get(11)?,
            notes: row.get(12)?,
            created_at: row.get(13)?,
            updated_at: row.get(14)?,
            current_price: None,
            market_value: None,
            pnl: None,
            pnl_pct: None,
        })
    })
    .map(Some)
    .or_else(|e| {
        if e == rusqlite::Error::QueryReturnedNoRows {
            Ok(None)
        } else {
            Err(e)
        }
    })
}

pub fn update(
    conn: &Connection,
    id: &str,
    name: Option<&str>,
    position_type: Option<&str>,
    entry_price: Option<f64>,
    shares: Option<f64>,
    leverage: Option<i32>,
    margin: Option<f64>,
    liquidation_price: Option<f64>,
    take_profit: Option<f64>,
    stop_loss: Option<f64>,
    exchange: Option<&str>,
    notes: Option<&str>,
) -> SqlResult<CryptoContract> {
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "UPDATE crypto_contract SET
            name = COALESCE(?, name),
            position_type = COALESCE(?, position_type),
            entry_price = COALESCE(?, entry_price),
            shares = COALESCE(?, shares),
            leverage = COALESCE(?, leverage),
            margin = COALESCE(?, margin),
            liquidation_price = ?,
            take_profit = ?,
            stop_loss = ?,
            exchange = ?,
            notes = ?,
            updated_at = ?
         WHERE id = ?",
        params![
            name,
            position_type,
            entry_price,
            shares,
            leverage,
            margin,
            liquidation_price,
            take_profit,
            stop_loss,
            exchange,
            notes,
            &now,
            id,
        ],
    )?;

    get(conn, id)?.ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)
}

pub fn delete(conn: &Connection, id: &str) -> SqlResult<()> {
    conn.execute("DELETE FROM crypto_contract WHERE id = ?", params![id])?;
    Ok(())
}
