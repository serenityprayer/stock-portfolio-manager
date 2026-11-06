//! External cash flow (deposit / withdraw) — design doc §7.3, option A.
//!
//! A flow record and the corresponding cash-holding adjustment are written in
//! the SAME transaction. This is essential: if only the flow were recorded,
//! `V(t)` would stay unchanged while `F(t)` changes, and the unit NAV would
//! show a phantom drawdown after every deposit.

use crate::commands::transactions::adjust_cash_holding;
use crate::db::Database;
use crate::models::risk::ExternalCashFlow;
use crate::services::exchange_rate_service::convert_currency;
use crate::services::risk_service::{load_rates, now_utc_str, BASE_CURRENCY};
use rusqlite::{params, OptionalExtension};

fn normalise_occurred_at(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return now_utc_str();
    }
    // Frontend DatePicker sends YYYY-MM-DD; anchor at midnight UTC so the
    // value stays lexicographically sortable with now_utc_str().
    if s.len() == 10 && s.as_bytes().get(4) == Some(&b'-') {
        return format!("{}T00:00:00Z", s);
    }
    s.to_string()
}

fn row_to_flow(r: &rusqlite::Row<'_>) -> rusqlite::Result<ExternalCashFlow> {
    Ok(ExternalCashFlow {
        id: r.get(0)?,
        account_id: r.get(1)?,
        currency: r.get(2)?,
        market: r.get(3)?,
        flow_type: r.get(4)?,
        amount: r.get(5)?,
        occurred_at: r.get(6)?,
        note: r.get(7)?,
        created_at: r.get(8)?,
    })
}

const FLOW_SELECT: &str = "SELECT id, account_id, currency, market, flow_type, amount, occurred_at, note, created_at FROM external_cash_flows";

pub fn list_external_flows(db: &Database) -> Result<Vec<ExternalCashFlow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(&format!("{} ORDER BY occurred_at DESC, created_at DESC LIMIT 200", FLOW_SELECT))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_flow)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn record_external_flow(
    db: &Database,
    account_id: &str,
    currency: &str,
    flow_type: &str,
    amount: f64,
    occurred_at: &str,
    note: Option<String>,
) -> Result<ExternalCashFlow, String> {
    if amount <= 0.0 || !amount.is_finite() {
        return Err("金额必须为正数".to_string());
    }
    let signed = match flow_type {
        "DEPOSIT" => amount,
        "WITHDRAW" => -amount,
        "DIVIDEND_ADJUST" => amount,
        other => return Err(format!("不支持的现金流类型: {}", other)),
    };

    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let market: String = conn
        .query_row(
            "SELECT market FROM accounts WHERE id = ?1",
            params![account_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "账户不存在".to_string())?;

    let rates = load_rates(&conn);
    let amount_base = convert_currency(signed, currency, BASE_CURRENCY, &rates);
    let now = now_utc_str();
    let occurred = normalise_occurred_at(occurred_at);
    let id = uuid::Uuid::new_v4().to_string();

    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO external_cash_flows
         (id, account_id, currency, market, flow_type, amount, amount_base,
          counted_in_base, occurred_at, note, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9, ?10)",
        params![
            id,
            account_id,
            currency,
            market,
            flow_type,
            signed,
            amount_base,
            occurred,
            note,
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    adjust_cash_holding(&tx, account_id, currency, &market, signed)?;
    tx.commit().map_err(|e| e.to_string())?;

    Ok(ExternalCashFlow {
        id,
        account_id: account_id.to_string(),
        currency: currency.to_string(),
        market,
        flow_type: flow_type.to_string(),
        amount: signed,
        occurred_at: occurred,
        note,
        created_at: now,
    })
}

pub fn delete_external_flow(db: &Database, id: &str) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let row: Option<(String, String, String, String, f64, i32)> = conn
        .query_row(
            "SELECT account_id, currency, market, flow_type, amount, counted_in_base
             FROM external_cash_flows WHERE id = ?1",
            params![id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let (account_id, currency, market, _flow_type, signed, counted) =
        row.ok_or_else(|| "现金流记录不存在".to_string())?;
    if counted != 0 {
        return Err("该记录已计入净值基准，不能删除（如需重算请使用 HWM 校准）".to_string());
    }

    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    adjust_cash_holding(&tx, &account_id, &currency, &market, -signed)?;
    tx.execute("DELETE FROM external_cash_flows WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}
