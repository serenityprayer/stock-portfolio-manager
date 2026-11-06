//! P1 — watchlist, cooling-off period, pre-trade check and discipline ledger.
//!
//! Design contract (docs/risk-control-system-design.md §6):
//!   * Only `BUY` is ever gated. `SELL` must never be blocked (it would prevent
//!     stop-losses and de-risking).
//!   * Existing positions are free to add to — the gate is about NEW positions.
//!   * A blocked decision can be overridden, but only with a written reason,
//!     and the decision is always recorded in `trade_discipline`.
//!
//! IMPORTANT: every public function takes `&Connection` and never locks
//! `Database` itself. `create_transaction` already holds the mutex and an open
//! `BEGIN IMMEDIATE`; calling `risk_service::build_context(db)` from there would
//! deadlock on the non-reentrant std::sync::Mutex.

use crate::models::risk::*;
use crate::services::risk_engine::{RULE_MAX_DRAWDOWN};
use crate::services::risk_service::now_utc_str;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};

const DEFAULT_COOLING_DAYS: f64 = 7.0;
const DEFAULT_DRAWDOWN_THRESHOLD: f64 = -5.0;

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    if s.len() >= 10 {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d") {
            let naive = d.and_hms_opt(0, 0, 0)?;
            return Some(Utc.from_utc_datetime(&naive));
        }
    }
    None
}

fn days_since(s: &str) -> f64 {
    parse_ts(s)
        .map(|dt| (Utc::now() - dt).num_seconds().max(0) as f64 / 86_400.0)
        .unwrap_or(0.0)
}

/// (enabled, threshold) for a rule; falls back to safe defaults when absent.
fn rule_config(conn: &Connection, id: &str) -> Result<(bool, f64), String> {
    let row: Option<(i32, Option<f64>)> = conn
        .query_row(
            "SELECT enabled, threshold FROM risk_rules WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let default_threshold = if id == RULE_MAX_DRAWDOWN {
        DEFAULT_DRAWDOWN_THRESHOLD
    } else {
        DEFAULT_COOLING_DAYS
    };
    Ok(match row {
        Some((enabled, threshold)) => (enabled != 0, threshold.unwrap_or(default_threshold)),
        None => (false, default_threshold),
    })
}

fn drawdown_pct(conn: &Connection) -> f64 {
    let row: Option<(f64, f64)> = conn
        .query_row(
            "SELECT peak_unit_nav, current_unit_nav FROM portfolio_hwm WHERE scope_key = 'GLOBAL'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    match row {
        Some((peak, current)) if peak > 0.0 => (current - peak) / peak * 100.0,
        _ => 0.0,
    }
}

fn has_position(conn: &Connection, symbol: &str) -> Result<bool, String> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM holdings WHERE UPPER(symbol) = UPPER(?1) AND shares > 0.00000001 LIMIT 1",
            params![symbol],
            |_| Ok(1i32),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .is_some())
}

fn cooling_threshold(conn: &Connection) -> Result<f64, String> {
    let (enabled, threshold) = rule_config(conn, RULE_COOLING_PERIOD)?;
    Ok(if enabled { threshold.max(0.0) } else { 0.0 })
}

fn row_to_watch(r: &rusqlite::Row<'_>, cooling_days_threshold: f64) -> rusqlite::Result<WatchItem> {
    let added_at: String = r.get(5)?;
    let cooling_days = days_since(&added_at);
    let days_remaining = (cooling_days_threshold - cooling_days).max(0.0);
    Ok(WatchItem {
        id: r.get(0)?,
        symbol: r.get(1)?,
        name: r.get(2)?,
        market: r.get(3)?,
        currency: r.get(4)?,
        added_at,
        thesis: r.get(6)?,
        target_price: r.get(7)?,
        status: r.get(8)?,
        promoted_at: r.get(9)?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
        cooling_days,
        days_remaining,
        eligible: cooling_days >= cooling_days_threshold,
    })
}

const WATCH_SELECT: &str = "SELECT id, symbol, name, market, currency, added_at, thesis,
                                   target_price, status, promoted_at, created_at, updated_at
                            FROM watchlist";

pub fn list_watchlist(conn: &Connection) -> Result<Vec<WatchItem>, String> {
    let threshold = cooling_threshold(conn)?;
    let mut stmt = conn
        .prepare(&format!(
            "{} WHERE status = 'watching' ORDER BY added_at DESC",
            WATCH_SELECT
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| row_to_watch(r, threshold))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// Add (or refresh) a watchlist entry. `added_at` is deliberately preserved on
/// conflict: re-adding a symbol must NOT reset its cooling-off clock.
pub fn add_watch_item(conn: &Connection, input: &WatchItemInput) -> Result<WatchItem, String> {
    let symbol = input.symbol.trim().to_uppercase();
    if symbol.is_empty() {
        return Err("标的代码不能为空".to_string());
    }
    let market = input.market.trim().to_string();
    if market.is_empty() {
        return Err("市场不能为空".to_string());
    }
    let now = now_utc_str();
    let added = input
        .added_at
        .as_deref()
        .map(|s| {
            if s.len() == 10 {
                format!("{}T00:00:00Z", s)
            } else {
                s.to_string()
            }
        })
        .unwrap_or_else(|| now.clone());
    let id = uuid::Uuid::new_v4().to_string();

    conn.execute(
        "INSERT INTO watchlist
         (id, symbol, name, market, currency, added_at, thesis, target_price,
          status, promoted_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'watching', NULL, ?9, ?9)
         ON CONFLICT(symbol, market) DO UPDATE SET
            name = excluded.name,
            thesis = excluded.thesis,
            target_price = excluded.target_price,
            updated_at = excluded.updated_at",
        params![
            id,
            symbol,
            input.name,
            market,
            input.currency,
            added,
            input.thesis,
            input.target_price,
            now
        ],
    )
    .map_err(|e| e.to_string())?;

    let threshold = cooling_threshold(conn)?;
    conn.query_row(
        &format!("{} WHERE UPPER(symbol) = UPPER(?1) AND market = ?2", WATCH_SELECT),
        params![symbol, market],
        |r| row_to_watch(r, threshold),
    )
    .map_err(|e| e.to_string())
}

pub fn remove_watch_item(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM watchlist WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Evaluate a trade intent. Pure read — safe to call inside an open transaction.
pub fn evaluate_trade_intent(
    conn: &Connection,
    symbol: &str,
    market: &str,
    action: &str,
) -> Result<TradeVerdict, String> {
    let base = |level: &str, message: String| TradeVerdict {
        level: level.to_string(),
        message,
        symbol: symbol.to_string(),
        action: action.to_string(),
        has_position: false,
        in_watchlist: false,
        cooling_days: None,
        days_remaining: None,
        is_unplanned: false,
        requires_reason: level == VERDICT_BLOCK,
        rule_ids: Vec::new(),
    };

    // Selling / dividends / contract OPEN are never gated.
    if action != "BUY" {
        return Ok(base(VERDICT_PASS, "卖出及其他交易不受冷静期约束".to_string()));
    }

    let held = has_position(conn, symbol)?;
    if held {
        let mut v = base(VERDICT_PASS, "已有持仓，加仓不受冷静期约束".to_string());
        v.has_position = true;
        return Ok(v);
    }

    let cooling_days_threshold = cooling_threshold(conn)?;
    let (cool_enabled, _) = rule_config(conn, RULE_COOLING_PERIOD)?;
    let (unplanned_enabled, _) = rule_config(conn, RULE_UNPLANNED_TRADE)?;

    let watch: Option<(String, String)> = conn
        .query_row(
            "SELECT id, added_at FROM watchlist
             WHERE UPPER(symbol) = UPPER(?1) AND market = ?2 AND status = 'watching'",
            params![symbol, market],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;

    let mut verdict = match watch {
        Some((_id, added_at)) => {
            let elapsed = days_since(&added_at);
            let remaining = (cooling_days_threshold - elapsed).max(0.0);
            let mut v = if cool_enabled && elapsed < cooling_days_threshold {
                base(
                    VERDICT_BLOCK,
                    format!(
                        "观察期未满：{} 已观察 {:.1} 天，还需 {:.1} 天（阈值 {:.0} 天）",
                        symbol, elapsed, remaining, cooling_days_threshold
                    ),
                )
            } else {
                base(
                    VERDICT_PASS,
                    format!("观察期已满 {:.1} 天，可以建仓", elapsed),
                )
            };
            v.in_watchlist = true;
            v.cooling_days = Some(elapsed);
            v.days_remaining = Some(remaining);
            if v.level == VERDICT_BLOCK {
                v.rule_ids.push(RULE_COOLING_PERIOD.to_string());
            }
            v
        }
        None => {
            let mut v = if unplanned_enabled {
                base(
                    VERDICT_WARN,
                    format!("{} 既无持仓也不在观察列表，属于非计划交易，请二次确认", symbol),
                )
            } else {
                base(VERDICT_PASS, "未启用非计划交易提醒".to_string())
            };
            v.is_unplanned = true;
            if v.level == VERDICT_WARN {
                v.rule_ids.push(RULE_UNPLANNED_TRADE.to_string());
            }
            v
        }
    };

    // Portfolio-level circuit breaker only blocks NEW positions (adding to an
    // existing holding is handled above). It can upgrade PASS/WARN to BLOCK.
    let (dd_enabled, dd_threshold) = rule_config(conn, RULE_MAX_DRAWDOWN)?;
    if dd_enabled {
        let dd = drawdown_pct(conn);
        if dd <= dd_threshold {
            verdict.level = VERDICT_BLOCK.to_string();
            verdict.message = format!(
                "组合回撤 {:.2}% 已触及熔断线 {:.2}%，禁止新开仓（可填写理由覆盖）",
                dd, dd_threshold
            );
            verdict.rule_ids.push(RULE_MAX_DRAWDOWN.to_string());
            verdict.requires_reason = true;
        }
    }

    Ok(verdict)
}

/// Persist the decision in the SAME transaction as the trade itself.
pub fn record_discipline(
    conn: &Connection,
    transaction_id: &str,
    verdict: &TradeVerdict,
    override_reason: Option<&str>,
) -> Result<(), String> {
    let now = now_utc_str();
    let rule_ids = serde_json::to_string(&verdict.rule_ids).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO trade_discipline
         (id, transaction_id, symbol, action, verdict, rule_ids, cooling_days,
          is_unplanned, override_reason, decided_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            uuid::Uuid::new_v4().to_string(),
            transaction_id,
            verdict.symbol,
            verdict.action,
            verdict.level,
            rule_ids,
            verdict.cooling_days,
            verdict.is_unplanned as i32,
            override_reason,
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Mark the watchlist entry as promoted after a successful BUY.
pub fn promote_watch_item(conn: &Connection, symbol: &str, market: &str) -> Result<(), String> {
    let now = now_utc_str();
    conn.execute(
        "UPDATE watchlist SET status = 'promoted', promoted_at = ?3, updated_at = ?3
         WHERE UPPER(symbol) = UPPER(?1) AND market = ?2 AND status = 'watching'",
        params![symbol, market, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE risk_rules (
                id TEXT PRIMARY KEY, name TEXT, description TEXT, severity TEXT,
                enabled INTEGER, threshold REAL, params TEXT, updated_at TEXT);
             CREATE TABLE watchlist (
                id TEXT PRIMARY KEY, symbol TEXT NOT NULL, name TEXT, market TEXT NOT NULL,
                currency TEXT, added_at TEXT NOT NULL, thesis TEXT, target_price REAL,
                status TEXT NOT NULL DEFAULT 'watching', promoted_at TEXT,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL, UNIQUE(symbol, market));
             CREATE TABLE holdings (
                id TEXT PRIMARY KEY, account_id TEXT, symbol TEXT, shares REAL);
             CREATE TABLE portfolio_hwm (
                scope_key TEXT PRIMARY KEY, peak_unit_nav REAL, current_unit_nav REAL);
             INSERT INTO risk_rules VALUES
                ('COOLING_PERIOD','冷静期','', 'block', 1, 7.0, NULL, ''),
                ('UNPLANNED_TRADE','非计划','', 'warning', 1, NULL, NULL, ''),
                ('MAX_DRAWDOWN','熔断','', 'block', 0, -5.0, NULL, '');
             INSERT INTO portfolio_hwm VALUES ('GLOBAL', 1.0, 1.0);",
        )
        .unwrap();
        conn
    }

    fn days_ago(n: i64) -> String {
        (Utc::now() - Duration::days(n))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    }

    fn watch(symbol: &str, added_at: String) -> WatchItemInput {
        WatchItemInput {
            symbol: symbol.to_string(),
            name: None,
            market: "US".to_string(),
            currency: Some("USD".to_string()),
            thesis: None,
            target_price: None,
            added_at: Some(added_at),
        }
    }

    #[test]
    fn sell_is_never_gated() {
        let conn = test_conn();
        let v = evaluate_trade_intent(&conn, "AAPL", "US", "SELL").unwrap();
        assert_eq!(v.level, VERDICT_PASS);
        assert!(!v.requires_reason);
    }

    #[test]
    fn adding_to_existing_position_passes() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO holdings VALUES ('h1','a1','AAPL',10.0)",
            [],
        )
        .unwrap();
        let v = evaluate_trade_intent(&conn, "AAPL", "US", "BUY").unwrap();
        assert_eq!(v.level, VERDICT_PASS);
        assert!(v.has_position);
    }

    #[test]
    fn buy_without_watchlist_is_warn() {
        let conn = test_conn();
        let v = evaluate_trade_intent(&conn, "NVDA", "US", "BUY").unwrap();
        assert_eq!(v.level, VERDICT_WARN);
        assert!(v.is_unplanned);
        assert!(!v.requires_reason);
        assert_eq!(v.rule_ids, vec![RULE_UNPLANNED_TRADE]);
    }

    #[test]
    fn cooling_period_blocks_and_reports_remaining() {
        let conn = test_conn();
        add_watch_item(&conn, &watch("TSLA", days_ago(2))).unwrap();
        let v = evaluate_trade_intent(&conn, "TSLA", "US", "BUY").unwrap();
        assert_eq!(v.level, VERDICT_BLOCK);
        assert!(v.requires_reason);
        assert!(v.in_watchlist);
        assert!(v.days_remaining.unwrap() > 4.9 && v.days_remaining.unwrap() <= 5.0);
        assert_eq!(v.rule_ids, vec![RULE_COOLING_PERIOD]);
    }

    #[test]
    fn cooling_period_complete_passes() {
        let conn = test_conn();
        add_watch_item(&conn, &watch("MSFT", days_ago(10))).unwrap();
        let v = evaluate_trade_intent(&conn, "MSFT", "US", "BUY").unwrap();
        assert_eq!(v.level, VERDICT_PASS);
        assert!(v.in_watchlist);
        assert_eq!(v.days_remaining, Some(0.0));
    }

    #[test]
    fn circuit_breaker_upgrades_even_when_cooling_complete() {
        let conn = test_conn();
        add_watch_item(&conn, &watch("META", days_ago(10))).unwrap();
        conn.execute(
            "UPDATE risk_rules SET enabled = 1 WHERE id = 'MAX_DRAWDOWN'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE portfolio_hwm SET peak_unit_nav = 1.0, current_unit_nav = 0.94",
            [],
        )
        .unwrap();
        let v = evaluate_trade_intent(&conn, "META", "US", "BUY").unwrap();
        assert_eq!(v.level, VERDICT_BLOCK);
        assert!(v.rule_ids.contains(&RULE_MAX_DRAWDOWN.to_string()));
        assert!(v.requires_reason);
    }

    #[test]
    fn disabled_cooling_rule_allows_immediate_buy() {
        let conn = test_conn();
        add_watch_item(&conn, &watch("AMD", days_ago(1))).unwrap();
        conn.execute(
            "UPDATE risk_rules SET enabled = 0 WHERE id = 'COOLING_PERIOD'",
            [],
        )
        .unwrap();
        let v = evaluate_trade_intent(&conn, "AMD", "US", "BUY").unwrap();
        assert_eq!(v.level, VERDICT_PASS);
    }

    #[test]
    fn re_adding_does_not_reset_cooling_clock() {
        let conn = test_conn();
        add_watch_item(&conn, &watch("COIN", days_ago(6))).unwrap();
        add_watch_item(&conn, &watch("COIN", days_ago(0))).unwrap();
        let items = list_watchlist(&conn).unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].cooling_days > 5.9);
    }
}

pub fn discipline_stats(conn: &Connection) -> Result<DisciplineStats, String> {
    let (total, blocked, warn, overridden, unplanned): (i64, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN verdict = 'BLOCK' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN verdict = 'WARN'  THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN override_reason IS NOT NULL AND override_reason != '' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN is_unplanned = 1 THEN 1 ELSE 0 END), 0)
             FROM trade_discipline",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .map_err(|e| e.to_string())?;
    let compliance_rate = if total > 0 {
        (total - overridden - warn) as f64 / total as f64 * 100.0
    } else {
        100.0
    };
    Ok(DisciplineStats {
        total,
        blocked,
        warn,
        overridden,
        unplanned,
        compliance_rate: compliance_rate.clamp(0.0, 100.0),
    })
}
