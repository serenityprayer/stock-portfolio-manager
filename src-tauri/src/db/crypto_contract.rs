use rusqlite::{params, Connection, Result};
use uuid::Uuid;
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

/// 加仓：在现有合约上追加仓位，重新计算加权平均开仓价
pub fn add_position(
    conn: &Connection,
    contract_id: &str,
    add_shares: f64,
    add_price: f64,
    add_fee: f64,
    updated_at: &str,
) -> Result<CryptoContract> {
    // 1. 读取原合约
    let mut stmt = conn.prepare(
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
    let contract = rows.next().transpose()?.ok_or_else(|| rusqlite::Error::InvalidQuery)?;

    // 2. 计算加权平均开仓价
    let old_notional = contract.open_price * contract.shares;
    let add_notional = add_price * add_shares;
    let total_shares = contract.shares + add_shares;
    let avg_open_price = (old_notional + add_notional) / total_shares;

    // 3. 累加手续费
    let total_fee = contract.fee.unwrap_or(0.0) + add_fee;

    // 4. 重新计算保证金
    let margin = (avg_open_price * total_shares) / contract.leverage + total_fee;

    // 5. 更新数据库
    conn.execute(
        "UPDATE crypto_contract SET
            open_price=?2, shares=?3, margin=?4, fee=?5, updated_at=?6
         WHERE id=?1",
        params![contract_id, avg_open_price, total_shares, margin, total_fee, updated_at],
    )?;

    // 6. 写入成交历史（加仓）
    let history_id_add = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO crypto_contract_close_history
         (id, contract_id, symbol, name, asset_type, position_type,
          open_price, action_type,
          add_price, add_shares, new_avg_price, new_total_shares,
          leverage, open_fee, close_fee, realized_pnl, closed_at, notes)
         VALUES (?1,?2,?3,?4,?5,?6,?7,'add',?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
        params![
            history_id_add,
            contract_id,
            contract.symbol,
            contract.name,
            contract.asset_type,
            contract.position_type,
            contract.open_price,
            add_price,
            add_shares,
            avg_open_price,
            total_shares,
            contract.leverage,
            contract.fee.unwrap_or(0.0),
            add_fee,
            0.0,
            updated_at,
            None::<String>,
        ],
    )?;

    // 7. 返回更新后的合约
    get_by_id(conn, contract_id)?
        .ok_or_else(|| rusqlite::Error::InvalidQuery)
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

    // 1. 读取原合约
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
        rows.next().transpose()?.ok_or_else(|| rusqlite::Error::InvalidQuery)?
    };

    let history_id = Uuid::new_v4().to_string();
    let realized_pnl = contract.calculate_pnl(close_price, close_shares, close_fee);

    // 计算回报率 = 盈亏 / 保证金
    let _margin_used = contract.open_price * close_shares / contract.leverage
        + contract.fee.unwrap_or(0.0)
        + close_fee;
    let _return_rate = if _margin_used.abs() > 1e-8 {
        realized_pnl / _margin_used
    } else {
        0.0
    };

    // 2. 写入平仓历史
    tx.execute(
        "INSERT INTO crypto_contract_close_history
         (id, contract_id, symbol, name, asset_type, position_type,
          open_price, close_price, close_shares, leverage,
          open_fee, close_fee, realized_pnl, return_rate, closed_at, notes,
          action_type)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,'close')",
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
            _return_rate,
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

/// 查询成交历史（平仓 + 加仓，按时间倒序）
pub fn list_contract_history(conn: &Connection) -> Result<Vec<serde_json::Value>> {
    let mut stmt = conn.prepare(
        "SELECT id, contract_id, symbol, name, asset_type, position_type,
                action_type, open_price, close_price, close_shares,
                add_price, add_shares, new_avg_price, new_total_shares,
                leverage, open_fee, close_fee, realized_pnl, return_rate,
                closed_at, notes
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
            "action_type": row.get::<_, String>(6)?,
            "open_price": row.get::<_, f64>(7)?,
            "close_price": row.get::<_, Option<f64>>(8)?,
            "close_shares": row.get::<_, Option<f64>>(9)?,
            "add_price": row.get::<_, Option<f64>>(10)?,
            "add_shares": row.get::<_, Option<f64>>(11)?,
            "new_avg_price": row.get::<_, Option<f64>>(12)?,
            "new_total_shares": row.get::<_, Option<f64>>(13)?,
            "leverage": row.get::<_, f64>(14)?,
            "open_fee": row.get::<_, f64>(15)?,
            "close_fee": row.get::<_, f64>(16)?,
            "realized_pnl": row.get::<_, Option<f64>>(17)?,
            "return_rate": row.get::<_, Option<f64>>(18)?,
            "closed_at": row.get::<_, String>(19)?,
            "notes": row.get::<_, Option<String>>(20)?,
        }))
    })?;
    rows.collect()
}
