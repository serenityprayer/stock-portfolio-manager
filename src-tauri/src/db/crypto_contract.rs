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

/// 平仓操作：
/// - 部分平仓：减少 shares，写入平仓历史
/// - 全部平仓：删除合约记录，写入平仓历史
/// 返回 (updated_contract_opt, history_id)
pub fn close_contract(
    conn: &mut Connection,
    contract_id: &str,
    close_price: f64,
    close_shares: f64,
    close_fee: f64,
    notes: Option<&str>,
    closed_at: &str,
) -> Result<(Option<CryptoContract>, String)> {
    use uuid::Uuid;

    let tx = conn.transaction()?;

    // 1. 读取原合约（放到独立作用域，让 stmt 先 drop）
    let contract = {
        let mut stmt = tx.prepare(
            "SELECT id, symbol, name, asset_type, position_type, open_price, shares,
                    leverage, margin, fee, exchange, notes, created_at, updated_at
             FROM crypto_contract WHERE id = ?1"
        )?;
        let mut rows = stmt.query_map(params![contract_id], |row| {
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
        rows.next().transpose()?
    }?;

    let history_id = Uuid::new_v4().to_string();
    let realized_pnl = contract.calculate_pnl(close_price, close_shares, close_fee);

    // 2. 写入平仓历史
    tx.execute(
        "INSERT INTO crypto_contract_close_history
         (id, contract_id, symbol, name, asset_type, position_type,
          open_price, close_price, close_shares, leverage,
          open_fee, close_fee, realized_pnl, closed_at, notes)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            history_id,
            contract_id,
            contract.symbol,
            contract.name,
            contract.asset_type,
            contract.position_type,
            contract.open_price,
            close_price,
            close_shares,
            contract.leverage,
            contract.fee.unwrap_or(0.0),
            close_fee,
            realized_pnl,
            closed_at,
            notes,
        ],
    )?;

    let remaining_shares = contract.shares - close_shares;

    if remaining_shares > 1e-8 {
        // 部分平仓：更新 shares 和 margin
        let new_margin = (contract.open_price * remaining_shares) / contract.leverage;
        tx.execute(
            "UPDATE crypto_contract SET shares=?2, margin=?3, updated_at=?4 WHERE id=?1",
            params![contract_id, remaining_shares, new_margin, closed_at],
        )?;

        // 重新读取更新后的合约
        let updated = get_by_id(&tx, contract_id)?;
        tx.commit()?;
        Ok((updated, history_id))
    } else {
        // 全部平仓：删除原合约
        tx.execute("DELETE FROM crypto_contract WHERE id = ?1", params![contract_id])?;
        tx.commit()?;
        Ok((None, history_id))
    }
}

/// 查询平仓历史（按平仓时间倒序）
pub fn list_close_history(conn: &Connection) -> Result<Vec<serde_json::Value>> {
    let mut stmt = conn.prepare(
        "SELECT id, contract_id, symbol, name, asset_type, position_type,
                open_price, close_price, close_shares, leverage,
                open_fee, close_fee, realized_pnl, closed_at, notes
         FROM crypto_contract_close_history
         ORDER BY closed_at DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, String>(0)?,
            "contract_id": row.get::<_, String>(1)?,
            "symbol": row.get::<_, String>(2)?,
            "name": row.get::<_, Option<String>>(3)?,
            "asset_type": row.get::<_, String>(4)?,
            "position_type": row.get::<_, String>(5)?,
            "open_price": row.get::<_, f64>(6)?,
            "close_price": row.get::<_, f64>(7)?,
            "close_shares": row.get::<_, f64>(8)?,
            "leverage": row.get::<_, f64>(9)?,
            "open_fee": row.get::<_, f64>(10)?,
            "close_fee": row.get::<_, f64>(11)?,
            "realized_pnl": row.get::<_, f64>(12)?,
            "closed_at": row.get::<_, String>(13)?,
            "notes": row.get::<_, Option<String>>(14)?,
        }))
    })?;
    rows.collect()
}
