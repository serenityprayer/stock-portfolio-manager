//! P2 — LIFO round trips, T+30 time capsules, discipline score, investment memos.
//!
//! Contract:
//!   * `pair_round_trips` is a PURE function (no IO) so it can be replayed and
//!     unit-tested against the real 24-row transaction history.
//!   * Every DB function takes `&Connection`, never `&Database`. Several of
//!     them are called from inside `create_transaction`, which already holds
//!     the mutex and an open `BEGIN IMMEDIATE` (std::sync::Mutex is not
//!     re-entrant — locking again would deadlock).
//!   * Snapshot truth is `holdings` + `cached_quotes`, never daily/quarterly
//!     snapshots (design doc §5.5).

use crate::models::reflection::*;
use crate::models::ExchangeRates;
use crate::services::exchange_rate_service::convert_currency;
use crate::services::quote_service::is_cash_symbol;
use crate::services::risk_service::{load_rates, now_utc_str, BASE_CURRENCY};
use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

const EPS: f64 = 1e-8;
const REVIEW_WINDOW_DAYS: i64 = 30;
const STOP_LOSS_ACTION_DAYS: i64 = 7;

fn parse_ts(s: &str) -> DateTime<Utc> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return dt.with_timezone(&Utc);
    }
    if let Ok(n) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Utc.from_utc_datetime(&n);
    }
    if s.len() >= 10 {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d") {
            if let Some(n) = d.and_hms_opt(0, 0, 0) {
                return Utc.from_utc_datetime(&n);
            }
        }
    }
    Utc::now()
}

#[derive(Debug, Clone)]
pub struct TxRow {
    pub id: String,
    pub account_id: String,
    pub symbol: String,
    pub market: String,
    pub transaction_type: String,
    pub shares: f64,
    pub price: f64,
    pub commission: f64,
    pub traded_at: String,
    pub notes: Option<String>,
    pub currency: String,
}

#[derive(Debug, Clone)]
struct Lot {
    shares: f64,
    price: f64,
    commission_per_share: f64,
    /// Native currency -> base currency factor captured for the open leg.
    fx: f64,
    open_at: DateTime<Utc>,
    open_at_str: String,
    note: Option<String>,
}

/// Native currency -> USD factor used for every aggregate statistic.
/// `ExchangeRates` holds the latest cached rates, not per-trade-day history;
/// the UI states this limitation via `RoundTripStats.fx_note`.
pub fn fx_to_base(currency: &str, rates: &ExchangeRates) -> f64 {
    let v = convert_currency(1.0, currency, BASE_CURRENCY, rates);
    if v.is_finite() && v > 0.0 { v } else { 1.0 }
}

/// LIFO pairing, grouped by (account, symbol). `OPEN` is an opening balance:
/// it participates in pairing but produces no cash flow of its own.
/// A single SELL that spans several lots yields ONE round trip with a
/// quantity-weighted average open price; `split_lots` records the split.
/// Both legs are converted to the base currency so mixed-market portfolios
/// can be aggregated safely.
pub fn pair_round_trips(txs: &[TxRow], rates: &ExchangeRates) -> (Vec<RoundTrip>, usize) {
    let mut groups: HashMap<(String, String), Vec<&TxRow>> = HashMap::new();
    for t in txs {
        if !matches!(t.transaction_type.as_str(), "BUY" | "OPEN" | "SELL") {
            continue; // PAY (dividend) is not a round trip leg
        }
        let key = (t.account_id.clone(), t.symbol.to_uppercase());
        groups.entry(key).or_default().push(t);
    }

    let mut out = Vec::new();
    let mut open_lot_count = 0usize;

    let mut keys: Vec<_> = groups.keys().cloned().collect();
    keys.sort();
    for key in keys {
        let mut rows = groups.remove(&key).unwrap_or_default();
        rows.sort_by(|a, b| {
            parse_ts(&a.traded_at)
                .cmp(&parse_ts(&b.traded_at))
                .then(a.id.cmp(&b.id))
        });

        let mut lots: Vec<Lot> = Vec::new();
        for tx in rows {
            match tx.transaction_type.as_str() {
                "BUY" | "OPEN" => {
                    if tx.shares <= EPS {
                        continue;
                    }
                    lots.push(Lot {
                        shares: tx.shares,
                        price: tx.price,
                        commission_per_share: tx.commission / tx.shares,
                        fx: fx_to_base(&tx.currency, rates),
                        open_at: parse_ts(&tx.traded_at),
                        open_at_str: tx.traded_at.clone(),
                        note: tx.notes.clone(),
                    });
                }
                "SELL" => {
                    let mut remaining = tx.shares;
                    let mut consumed = 0.0f64;
                    let mut open_cost = 0.0f64;
                    let mut open_commission = 0.0f64;
                    let mut open_cost_base = 0.0f64;
                    let mut open_commission_base = 0.0f64;
                    let mut lots_used = 0usize;
                    let mut open_at: Option<DateTime<Utc>> = None;
                    let mut open_at_str = tx.traded_at.clone();
                    let mut open_note: Option<String> = None;

                    while remaining > EPS && !lots.is_empty() {
                        let lot = lots.last_mut().unwrap();
                        let take = remaining.min(lot.shares);
                        if take <= EPS {
                            break;
                        }
                        if lots_used == 0 {
                            // LIFO: the newest lot determines the holding period.
                            open_at = Some(lot.open_at);
                            open_at_str = lot.open_at_str.clone();
                            open_note = lot.note.clone();
                        }
                        lots_used += 1;
                        open_cost += take * lot.price;
                        open_commission += take * lot.commission_per_share;
                        open_cost_base += take * lot.price * lot.fx;
                        open_commission_base += take * lot.commission_per_share * lot.fx;
                        lot.shares -= take;
                        remaining -= take;
                        consumed += take;
                        if lot.shares <= EPS {
                            lots.pop();
                        }
                    }
                    if consumed <= EPS {
                        continue; // oversold / no matching lot — ignore defensively
                    }
                    let close_commission = tx.commission * (consumed / tx.shares.max(EPS));
                    let cost_basis = open_cost + open_commission;
                    let proceeds = tx.price * consumed;
                    let pnl = proceeds - close_commission - cost_basis;
                    let sell_fx = fx_to_base(&tx.currency, rates);
                    let pnl_base = proceeds * sell_fx - close_commission * sell_fx
                        - open_cost_base - open_commission_base;
                    let return_rate = if cost_basis > EPS {
                        pnl / cost_basis * 100.0
                    } else {
                        0.0
                    };
                    let holding_days = open_at
                        .map(|o| (parse_ts(&tx.traded_at) - o).num_days().max(0))
                        .unwrap_or(0);
                    out.push(RoundTrip {
                        symbol: tx.symbol.to_uppercase(),
                        market: tx.market.clone(),
                        currency: tx.currency.clone(),
                        open_at: open_at_str,
                        close_at: tx.traded_at.clone(),
                        holding_days,
                        qty: consumed,
                        avg_open_price: open_cost / consumed,
                        avg_close_price: tx.price,
                        realized_pnl: pnl,
                        realized_pnl_base: pnl_base,
                        return_rate,
                        open_note,
                        close_note: tx.notes.clone(),
                        split_lots: lots_used > 1,
                    });
                }
                _ => {}
            }
        }
        open_lot_count += lots.len();
    }

    out.sort_by(|a, b| a.close_at.cmp(&b.close_at).then(a.symbol.cmp(&b.symbol)));
    (out, open_lot_count)
}

pub fn summarize(trips: &[RoundTrip], open_lot_count: usize) -> RoundTripStats {
    let wins: Vec<&RoundTrip> = trips.iter().filter(|t| t.realized_pnl_base > 0.0).collect();
    let losses: Vec<&RoundTrip> = trips.iter().filter(|t| t.realized_pnl_base < 0.0).collect();
    let gross_profit: f64 = wins.iter().map(|t| t.realized_pnl_base).sum();
    let gross_loss: f64 = losses.iter().map(|t| t.realized_pnl_base.abs()).sum();
    let profit_factor = if gross_loss > EPS {
        Some(gross_profit / gross_loss)
    } else if gross_profit > EPS {
        None // no losing trade → report "—" instead of infinity
    } else {
        None
    };
    RoundTripStats {
        total_trades: trips.len(),
        winning_trades: wins.len(),
        losing_trades: losses.len(),
        win_rate: if trips.is_empty() {
            0.0
        } else {
            wins.len() as f64 / trips.len() as f64 * 100.0
        },
        profit_factor,
        total_realized_pnl: trips.iter().map(|t| t.realized_pnl).sum(),
        total_realized_pnl_base: trips.iter().map(|t| t.realized_pnl_base).sum(),
        base_currency: BASE_CURRENCY.to_string(),
        mixed_currency: trips
            .iter()
            .map(|t| t.currency.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len()
            > 1,
        fx_note: "按当前缓存汇率折算到 USD；未保留成交日历史汇率".to_string(),
        avg_holding_days: if trips.is_empty() {
            0.0
        } else {
            trips.iter().map(|t| t.holding_days as f64).sum::<f64>() / trips.len() as f64
        },
        best_trade: trips
            .iter()
            .max_by(|a, b| {
                a.realized_pnl_base
                    .partial_cmp(&b.realized_pnl_base)
                    .unwrap()
            })
            .cloned(),
        worst_trade: trips
            .iter()
            .min_by(|a, b| {
                a.realized_pnl_base
                    .partial_cmp(&b.realized_pnl_base)
                    .unwrap()
            })
            .cloned(),
        open_lot_count,
    }
}

// ────────────────────────────────────────────────────────────────
// Database access
// ────────────────────────────────────────────────────────────────

pub fn load_transactions(conn: &Connection, symbol: Option<&str>) -> Result<Vec<TxRow>, String> {
    let sql = match symbol {
        Some(_) => "SELECT id, account_id, symbol, market, transaction_type, shares, price,
                           commission, traded_at, notes, currency FROM transactions
                    WHERE UPPER(symbol) = UPPER(?1) ORDER BY traded_at, id",
        None => "SELECT id, account_id, symbol, market, transaction_type, shares, price,
                        commission, traded_at, notes, currency FROM transactions ORDER BY traded_at, id",
    };
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let map = |r: &rusqlite::Row<'_>| -> rusqlite::Result<TxRow> {
        Ok(TxRow {
            id: r.get(0)?,
            account_id: r.get(1)?,
            symbol: r.get(2)?,
            market: r.get(3)?,
            transaction_type: r.get(4)?,
            shares: r.get(5)?,
            price: r.get(6)?,
            commission: r.get(7)?,
            traded_at: r.get(8)?,
            notes: r.get(9)?,
            currency: r.get(10)?,
        })
    };
    let rows = match symbol {
        Some(s) => stmt.query_map(params![s], map),
        None => stmt.query_map([], map),
    }
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn list_round_trips(
    conn: &Connection,
    symbol: Option<&str>,
) -> Result<Vec<RoundTrip>, String> {
    let txs = load_transactions(conn, symbol)?;
    let rates = load_rates(conn);
    Ok(pair_round_trips(&txs, &rates).0)
}

pub fn round_trip_summary(
    conn: &Connection,
    symbol: Option<&str>,
) -> Result<RoundTripStats, String> {
    let txs = load_transactions(conn, symbol)?;
    let rates = load_rates(conn);
    let (trips, open_lots) = pair_round_trips(&txs, &rates);
    Ok(summarize(&trips, open_lots))
}

/// Idempotent: one `time_capsule` per transaction. Called inside the same
/// SQLite transaction as the trade insert.
pub fn ensure_time_capsule(
    conn: &Connection,
    transaction_id: &str,
    symbol: &str,
    traded_at: &str,
) -> Result<bool, String> {
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM trade_reviews WHERE transaction_id = ?1
             AND review_type = 'time_capsule' LIMIT 1",
            params![transaction_id],
            |_| Ok(1i32),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .is_some();
    if exists {
        return Ok(false);
    }
    let due = (parse_ts(traded_at) + Duration::days(REVIEW_WINDOW_DAYS))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let now = now_utc_str();
    conn.execute(
        "INSERT INTO trade_reviews
         (id, transaction_id, symbol, review_type, decision_quality, content,
          citations, due_at, completed_at, created_at)
         VALUES (?1, ?2, ?3, 'time_capsule', NULL, NULL, NULL, ?4, NULL, ?5)",
        params![uuid::Uuid::new_v4().to_string(), transaction_id, symbol, due, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Backfill historical BUY/OPEN trades. Their due date is already in the past,
/// which is intentional: they enter the review queue immediately.
pub fn backfill_time_capsules(conn: &Connection) -> Result<usize, String> {
    let rows: Vec<(String, String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT t.id, t.symbol, t.traded_at, t.transaction_type FROM transactions t
                 WHERE t.transaction_type IN ('BUY', 'OPEN')
                   AND NOT EXISTS (
                     SELECT 1 FROM trade_reviews r
                     WHERE r.transaction_id = t.id AND r.review_type = 'time_capsule')
                 ORDER BY t.traded_at",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    let mut count = 0usize;
    for (id, symbol, traded_at, _kind) in rows {
        if ensure_time_capsule(conn, &id, &symbol, &traded_at)? {
            count += 1;
        }
    }
    Ok(count)
}

pub fn list_due_reviews(
    conn: &Connection,
    include_completed: bool,
) -> Result<Vec<DueReview>, String> {
    // Two literal statements keep the SQL-binding guard applicable to 100% of
    // the code: no format!() fragment with unresolvable placeholders.
    let sql = if include_completed {
        "SELECT r.id, r.transaction_id, r.symbol, r.review_type, r.decision_quality,
                r.content, r.due_at, r.completed_at, r.created_at,
                t.traded_at, t.notes, COALESCE(t.market, ''),
                (SELECT td.override_reason FROM trade_discipline td
                  WHERE td.transaction_id = r.transaction_id
                    AND td.override_reason IS NOT NULL AND td.override_reason != ''
                  ORDER BY td.decided_at DESC LIMIT 1)
         FROM trade_reviews r
         LEFT JOIN transactions t ON t.id = r.transaction_id
         ORDER BY (r.completed_at IS NOT NULL) ASC,
                  ((SELECT td.override_reason FROM trade_discipline td
                     WHERE td.transaction_id = r.transaction_id
                       AND td.override_reason IS NOT NULL AND td.override_reason != ''
                     ORDER BY td.decided_at DESC LIMIT 1) IS NOT NULL) DESC,
                  COALESCE(r.due_at, '9999') ASC"
    } else {
        "SELECT r.id, r.transaction_id, r.symbol, r.review_type, r.decision_quality,
                r.content, r.due_at, r.completed_at, r.created_at,
                t.traded_at, t.notes, COALESCE(t.market, ''),
                (SELECT td.override_reason FROM trade_discipline td
                  WHERE td.transaction_id = r.transaction_id
                    AND td.override_reason IS NOT NULL AND td.override_reason != ''
                  ORDER BY td.decided_at DESC LIMIT 1)
         FROM trade_reviews r
         LEFT JOIN transactions t ON t.id = r.transaction_id
         WHERE r.completed_at IS NULL
         ORDER BY (r.completed_at IS NOT NULL) ASC,
                  ((SELECT td.override_reason FROM trade_discipline td
                     WHERE td.transaction_id = r.transaction_id
                       AND td.override_reason IS NOT NULL AND td.override_reason != ''
                     ORDER BY td.decided_at DESC LIMIT 1) IS NOT NULL) DESC,
                  COALESCE(r.due_at, '9999') ASC"
    };
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let now = Utc::now();
    let rows = stmt
        .query_map([], |r| {
            let due_at: Option<String> = r.get(6)?;
            let override_reason: Option<String> = r.get(12)?;
            let days_overdue = due_at
                .as_ref()
                .map(|d| (now - parse_ts(d)).num_days().max(0))
                .unwrap_or(0);
            Ok(DueReview {
                id: r.get(0)?,
                transaction_id: r.get(1)?,
                symbol: r.get(2)?,
                market: r.get(11)?,
                review_type: r.get(3)?,
                decision_quality: r.get(4)?,
                content: r.get(5)?,
                due_at,
                completed_at: r.get(7)?,
                created_at: r.get(8)?,
                traded_at: r.get(9)?,
                open_note: r.get(10)?,
                is_priority: override_reason
                    .as_ref()
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false),
                override_reason,
                days_overdue,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn complete_review(conn: &Connection, input: &ReviewCompleteInput) -> Result<(), String> {
    if !matches!(
        input.quality.as_str(),
        QUALITY_CORRECT | QUALITY_WRONG | QUALITY_PENDING
    ) {
        return Err("决策质量只能是 correct / wrong / pending".to_string());
    }
    let now = now_utc_str();
    let completed = if input.quality == QUALITY_PENDING {
        None
    } else {
        Some(now.as_str())
    };
    let rows = conn
        .execute(
            "UPDATE trade_reviews SET decision_quality = ?2, content = ?3,
             completed_at = ?4 WHERE id = ?1",
            params![input.id, input.quality, input.content, completed],
        )
        .map_err(|e| e.to_string())?;
    if rows == 0 {
        return Err("复盘记录不存在".to_string());
    }
    Ok(())
}

// ────────────────────────────────────────────────────────────────
// Discipline score (§8.3) — trend display only, never a punishment.
// ────────────────────────────────────────────────────────────────

fn ratio(numer: i64, denom: i64) -> Option<f64> {
    if denom <= 0 {
        None
    } else {
        Some((numer as f64 / denom as f64 * 100.0).clamp(0.0, 100.0))
    }
}

pub fn discipline_score(conn: &Connection) -> Result<DisciplineScore, String> {
    // 新开仓 = 手动 BUY 中走观察列表或非计划路径的记录；加仓为 PASS 且无标记。
    let (new_positions, overridden, planned_buys, unplanned): (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT
               COALESCE(SUM(CASE WHEN action='BUY' AND (is_unplanned=1 OR cooling_days IS NOT NULL)
                                THEN 1 ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN override_reason IS NOT NULL AND override_reason != ''
                                THEN 1 ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN action='BUY' THEN 1 ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN action='BUY' AND is_unplanned=1 THEN 1 ELSE 0 END), 0)
             FROM trade_discipline",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|e| e.to_string())?;
    let cooling = ratio(new_positions - overridden.min(new_positions), new_positions);
    let planned = ratio(planned_buys - unplanned, planned_buys);

    // 止损执行率：触发 STOP_LOSS 后 7 日内是否有减/清仓。
    let events: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT COALESCE(symbol,''), first_triggered_at FROM risk_events
                 WHERE rule_id = 'STOP_LOSS' AND symbol IS NOT NULL",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    let sells: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT UPPER(symbol), traded_at FROM transactions WHERE transaction_type='SELL'")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    let mut acted = 0i64;
    let mut triggered = 0i64;
    for (sym, at) in &events {
        if sym.is_empty() {
            continue;
        }
        triggered += 1;
        let t0 = parse_ts(at);
        let t1 = t0 + Duration::days(STOP_LOSS_ACTION_DAYS);
        if sells.iter().any(|(s, ts)| {
            s.eq_ignore_ascii_case(sym) && {
                let t = parse_ts(ts);
                t >= t0 && t <= t1
            }
        }) {
            acted += 1;
        }
    }
    let stop_loss = ratio(acted, triggered);

    let now = now_utc_str();
    let (due_total, due_done): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(CASE WHEN completed_at IS NOT NULL THEN 1 ELSE 0 END), 0)
             FROM trade_reviews
             WHERE review_type = 'time_capsule' AND due_at IS NOT NULL AND due_at <= ?1",
            params![now],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let review = ratio(due_done, due_total);

    // 样本不足的分项不计入加权，权重在可用项之间重新归一化。
    const WEIGHTS: [(&str, f64); 4] = [
        ("cooling", 0.30),
        ("planned", 0.25),
        ("stop", 0.25),
        ("review", 0.20),
    ];
    let parts = [cooling, planned, stop_loss, review];
    let mut weight_sum = 0.0f64;
    let mut weighted = 0.0f64;
    for ((_, w), part) in WEIGHTS.iter().zip(parts.iter()) {
        if let Some(v) = part {
            weight_sum += w;
            weighted += w * v;
        }
    }
    let score = if weight_sum > 0.0 {
        weighted / weight_sum
    } else {
        100.0
    };
    let sample_size = planned_buys + triggered + due_total;
    Ok(DisciplineScore {
        score: score.clamp(0.0, 100.0),
        cooling_compliance: cooling,
        planned_ratio: planned,
        stop_loss_execution: stop_loss,
        review_completion: review,
        cooling_detail: format!("{}/{} 次新建仓未被破例覆盖", new_positions - overridden.min(new_positions), new_positions),
        planned_detail: format!("{}/{} 笔买入属于计划内", planned_buys - unplanned, planned_buys),
        stop_loss_detail: format!("{}/{} 次止损预警后 7 日内减仓", acted, triggered),
        review_detail: format!("{}/{} 笔到期复盘已完成", due_done, due_total),
        sample_size,
        low_sample: sample_size < 5,
    })
}

// ────────────────────────────────────────────────────────────────
// Investment memos + frozen snapshot
// ────────────────────────────────────────────────────────────────

pub fn build_snapshot(conn: &Connection) -> Result<HoldingsSnapshot, String> {
    let rates: ExchangeRates = load_rates(conn);
    // cached_quotes 里存在大小写重复行（TCOM / tcom），按 updated_at 取最新，
    // 与 risk_service::build_context 的口径保持一致。
    let mut quotes: HashMap<String, (f64, String)> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT symbol, current_price, updated_at FROM cached_quotes")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, String>(2)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (sym, price, ts) in rows {
            let key = sym.to_lowercase();
            let fresh = quotes.get(&key).map(|(_, old)| ts > *old).unwrap_or(true);
            if fresh {
                quotes.insert(key, (price.max(0.0), ts));
            }
        }
    }

    let mut cash_value_base = 0.0f64;
    let mut total_assets_base = 0.0f64;
    let mut positions: Vec<SnapshotPosition> = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT symbol, name, shares, avg_cost, currency FROM holdings
                 WHERE shares != 0 ORDER BY symbol",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, f64>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (symbol, name, shares, avg_cost, currency) in rows {
            let native = if is_cash_symbol(&symbol) {
                shares // cash: price is always 1
            } else {
                let price = quotes
                    .get(&symbol.to_lowercase())
                    .map(|(p, _)| *p)
                    .filter(|p| *p > 0.0)
                    .unwrap_or(avg_cost);
                positions.push(SnapshotPosition {
                    symbol: symbol.clone(),
                    name,
                    shares,
                    avg_cost,
                    price,
                    market_value_base: 0.0,
                    weight: 0.0,
                    has_quote: false,
                });
                shares * price
            };
            let mv = convert_currency(native, &currency, BASE_CURRENCY, &rates);
            if is_cash_symbol(&symbol) {
                cash_value_base += mv;
            } else if let Some(p) = positions.last_mut() {
                p.has_quote = quotes
                    .get(&symbol.to_lowercase())
                    .map(|(v, _)| *v > 0.0)
                    .unwrap_or(false);
                p.market_value_base = mv;
            }
            total_assets_base += mv;
        }
    }
    if total_assets_base > EPS {
        for p in &mut positions {
            p.weight = p.market_value_base / total_assets_base * 100.0;
        }
    }

    let (unit_nav, peak_unit_nav): (f64, f64) = conn
        .query_row(
            "SELECT current_unit_nav, peak_unit_nav FROM portfolio_hwm WHERE scope_key='GLOBAL'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or((1.0, 1.0));

    let active_events: Vec<SnapshotEvent> = {
        let mut stmt = conn
            .prepare(
                "SELECT rule_id, symbol, severity, message FROM risk_events
                 WHERE status IN ('active','acknowledged')
                 ORDER BY CASE severity WHEN 'block' THEN 0 WHEN 'critical' THEN 1
                          WHEN 'warning' THEN 2 ELSE 3 END, last_triggered_at DESC
                 LIMIT 20",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SnapshotEvent {
                    rule_id: r.get(0)?,
                    symbol: r.get(1)?,
                    severity: r.get(2)?,
                    message: r.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };

    Ok(HoldingsSnapshot {
        as_of: now_utc_str(),
        base_currency: BASE_CURRENCY.to_string(),
        total_assets_base,
        cash_value_base,
        unit_nav,
        peak_unit_nav,
        positions,
        active_events,
    })
}

fn row_to_memo(r: &rusqlite::Row<'_>) -> rusqlite::Result<InvestmentMemo> {
    Ok(InvestmentMemo {
        id: r.get(0)?,
        title: r.get(1)?,
        content: r.get(2)?,
        tags: r.get(3)?,
        holdings_snapshot: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

const MEMO_LIST: &str = "SELECT id, title, content, tags, holdings_snapshot, created_at, updated_at FROM investment_memos ORDER BY created_at DESC";
const MEMO_BY_ID: &str = "SELECT id, title, content, tags, holdings_snapshot, created_at, updated_at FROM investment_memos WHERE id = ?1";

pub fn list_memos(conn: &Connection) -> Result<Vec<InvestmentMemo>, String> {
    let mut stmt = conn
        .prepare(MEMO_LIST)
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_memo)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn create_memo(conn: &Connection, input: &MemoInput) -> Result<InvestmentMemo, String> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err("笔记标题不能为空".to_string());
    }
    let snapshot = build_snapshot(conn)?;
    let snapshot_json = serde_json::to_string(&snapshot).map_err(|e| e.to_string())?;
    let now = now_utc_str();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO investment_memos
         (id, title, content, tags, holdings_snapshot, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![id, title, input.content, input.tags, snapshot_json, now],
    )
    .map_err(|e| e.to_string())?;
    conn.query_row(
        MEMO_BY_ID,
        params![id],
        row_to_memo,
    )
    .map_err(|e| e.to_string())
}

pub fn delete_memo(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM investment_memos WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn compare_memo(conn: &Connection, id: &str) -> Result<MemoComparison, String> {
    let memo = conn
        .query_row(
            MEMO_BY_ID,
            params![id],
            row_to_memo,
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "笔记不存在".to_string())?;
    let then_snapshot: Option<HoldingsSnapshot> = memo
        .holdings_snapshot
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    let now_snapshot = build_snapshot(conn)?;
    let mut changes = Vec::new();
    if let Some(then) = &then_snapshot {
        let now_map: HashMap<&str, &SnapshotPosition> = now_snapshot
            .positions
            .iter()
            .map(|p| (p.symbol.as_str(), p))
            .collect();
        for p in &then.positions {
            let n = now_map.get(p.symbol.as_str()).copied();
            changes.push(MemoPositionChange {
                symbol: p.symbol.clone(),
                then_weight: p.weight,
                now_weight: n.map(|x| x.weight).unwrap_or(0.0),
                then_value: p.market_value_base,
                now_value: n.map(|x| x.market_value_base).unwrap_or(0.0),
                value_change: n.map(|x| x.market_value_base).unwrap_or(0.0)
                    - p.market_value_base,
            });
        }
    }
    Ok(MemoComparison {
        memo,
        then_snapshot,
        now_snapshot,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(id: &str, kind: &str, shares: f64, price: f64, commission: f64, at: &str) -> TxRow {
        TxRow {
            id: id.to_string(),
            account_id: "a1".to_string(),
            symbol: "TEST".to_string(),
            market: "US".to_string(),
            transaction_type: kind.to_string(),
            shares,
            price,
            commission,
            traded_at: at.to_string(),
            notes: Some(format!("note-{id}")),
            currency: "USD".to_string(),
        }
    }

    fn rates() -> ExchangeRates {
        ExchangeRates {
            usd_cny: 7.2,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / 7.2,
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn lifo_matches_newest_lot_first() {
        let txs = vec![
            tx("1", "BUY", 10.0, 10.0, 0.0, "2026-01-01T00:00:00Z"),
            tx("2", "BUY", 10.0, 20.0, 0.0, "2026-02-01T00:00:00Z"),
            tx("3", "SELL", 10.0, 25.0, 0.0, "2026-03-01T00:00:00Z"),
        ];
        let (trips, open_lots) = pair_round_trips(&txs, &rates());
        assert_eq!(trips.len(), 1);
        assert_eq!(open_lots, 1);
        assert!((trips[0].avg_open_price - 20.0).abs() < 1e-9, "LIFO 应优先匹配 20 元的批次");
        assert!((trips[0].realized_pnl - 50.0).abs() < 1e-9);
        assert_eq!(trips[0].holding_days, 28);
        assert_eq!(trips[0].open_note.as_deref(), Some("note-2"));
    }

    #[test]
    fn partial_sell_leaves_remainder_in_lot() {
        let txs = vec![
            tx("1", "OPEN", 100.0, 5.0, 0.0, "2026-01-01T00:00:00Z"),
            tx("2", "SELL", 30.0, 8.0, 0.0, "2026-02-01T00:00:00Z"),
            tx("3", "SELL", 20.0, 9.0, 0.0, "2026-03-01T00:00:00Z"),
        ];
        let (trips, open_lots) = pair_round_trips(&txs, &rates());
        assert_eq!(trips.len(), 2);
        assert_eq!(open_lots, 1, "剩余 50 股仍在未平仓批次");
        assert!((trips[0].realized_pnl - 90.0).abs() < 1e-9);
        assert!((trips[1].realized_pnl - 80.0).abs() < 1e-9);
        assert!(!trips[0].split_lots);
    }

    #[test]
    fn sell_across_lots_is_one_round_trip() {
        let txs = vec![
            tx("1", "BUY", 5.0, 10.0, 0.0, "2026-01-01T00:00:00Z"),
            tx("2", "BUY", 5.0, 20.0, 0.0, "2026-02-01T00:00:00Z"),
            tx("3", "SELL", 10.0, 30.0, 0.0, "2026-03-01T00:00:00Z"),
        ];
        let (trips, open_lots) = pair_round_trips(&txs, &rates());
        assert_eq!(trips.len(), 1);
        assert_eq!(open_lots, 0);
        assert!(trips[0].split_lots);
        assert!((trips[0].avg_open_price - 15.0).abs() < 1e-9);
        assert!((trips[0].realized_pnl - 150.0).abs() < 1e-9);
    }

    #[test]
    fn commission_reduces_pnl_and_pay_is_not_a_leg() {
        let txs = vec![
            tx("1", "BUY", 10.0, 10.0, 10.0, "2026-01-01T00:00:00Z"),
            tx("2", "PAY", 0.0, 0.0, 0.0, "2026-01-15T00:00:00Z"),
            tx("3", "SELL", 10.0, 12.0, 5.0, "2026-02-01T00:00:00Z"),
        ];
        let (trips, _) = pair_round_trips(&txs, &rates());
        assert_eq!(trips.len(), 1);
        // proceeds 120 - close fee 5 - (100 + open fee 10) = 5
        assert!((trips[0].realized_pnl - 5.0).abs() < 1e-9);
    }

    #[test]
    fn stats_cover_win_rate_and_profit_factor() {
        let txs = vec![
            tx("1", "BUY", 10.0, 10.0, 0.0, "2026-01-01T00:00:00Z"),
            tx("2", "SELL", 5.0, 20.0, 0.0, "2026-02-01T00:00:00Z"), // +50
            tx("3", "SELL", 5.0, 6.0, 0.0, "2026-03-01T00:00:00Z"),  // -20
        ];
        let (trips, open_lots) = pair_round_trips(&txs, &rates());
        let s = summarize(&trips, open_lots);
        assert_eq!(s.total_trades, 2);
        assert_eq!(s.winning_trades, 1);
        assert_eq!(s.losing_trades, 1);
        assert!((s.win_rate - 50.0).abs() < 1e-9);
        assert!((s.profit_factor.unwrap() - 2.5).abs() < 1e-9);
        assert!((s.total_realized_pnl - 30.0).abs() < 1e-9);
        assert_eq!(s.open_lot_count, 0);
    }

    #[test]
    fn groups_are_isolated_by_symbol_and_account() {
        let mut a = tx("1", "BUY", 10.0, 10.0, 0.0, "2026-01-01T00:00:00Z");
        a.symbol = "AAA".into();
        let mut b = tx("2", "BUY", 10.0, 10.0, 0.0, "2026-01-01T00:00:00Z");
        b.symbol = "BBB".into();
        let mut c = tx("3", "SELL", 10.0, 15.0, 0.0, "2026-02-01T00:00:00Z");
        c.symbol = "BBB".into();
        let (trips, open_lots) = pair_round_trips(&[a, b, c], &rates());
        assert_eq!(trips.len(), 1);
        assert_eq!(trips[0].symbol, "BBB");
        assert_eq!(open_lots, 1, "AAA 仍有未平仓批次");
    }

    #[test]
    fn hkd_round_trip_is_converted_before_aggregation() {
        let mut buy = tx("1", "BUY", 10.0, 100.0, 0.0, "2026-01-01T00:00:00Z");
        buy.currency = "HKD".into();
        let mut sell = tx("2", "SELL", 10.0, 120.0, 0.0, "2026-02-01T00:00:00Z");
        sell.currency = "HKD".into();
        let (trips, _) = pair_round_trips(&[buy, sell], &rates());
        assert_eq!(trips.len(), 1);
        assert!((trips[0].realized_pnl - 200.0).abs() < 1e-9, "原币盈亏仍为 200 HKD");
        assert!((trips[0].realized_pnl_base - 200.0 / 7.8).abs() < 1e-9);
        let s = summarize(&trips, 0);
        assert!((s.total_realized_pnl_base - 200.0 / 7.8).abs() < 1e-9);
        assert_eq!(s.base_currency, "USD");
        assert!(!s.mixed_currency);
    }

    #[test]
    fn mixed_currency_stats_never_sum_native_amounts() {
        let mut a = tx("1", "BUY", 1.0, 10.0, 0.0, "2026-01-01T00:00:00Z");
        a.currency = "USD".into();
        a.symbol = "AAA".into();
        let mut b = tx("2", "SELL", 1.0, 20.0, 0.0, "2026-02-01T00:00:00Z");
        b.currency = "USD".into();
        b.symbol = "AAA".into();
        let mut c = tx("3", "BUY", 1.0, 100.0, 0.0, "2026-01-01T00:00:00Z");
        c.currency = "HKD".into();
        c.symbol = "BBB".into();
        let mut d = tx("4", "SELL", 1.0, 110.0, 0.0, "2026-02-01T00:00:00Z");
        d.currency = "HKD".into();
        d.symbol = "BBB".into();
        let (trips, _) = pair_round_trips(&[a, b, c, d], &rates());
        let s = summarize(&trips, 0);
        assert!(s.mixed_currency);
        assert!((s.total_realized_pnl_base - (10.0 + 10.0 / 7.8)).abs() < 1e-9);
        assert!((s.total_realized_pnl - 20.0).abs() < 1e-9, "原币合计仅作参考");
    }

    #[test]
    fn empty_history_is_safe() {
        let (trips, open_lots) = pair_round_trips(&[], &rates());
        let s = summarize(&trips, open_lots);
        assert_eq!(s.total_trades, 0);
        assert_eq!(s.win_rate, 0.0);
        assert!(s.profit_factor.is_none());
    }
}
