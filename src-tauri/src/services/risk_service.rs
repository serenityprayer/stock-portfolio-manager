//! Orchestration layer for the Risk & Discipline Engine.
//!
//! Data truth: `holdings` (live) + `cached_quotes` + `cached_exchange_rates`.
//! Daily/quarterly snapshots are NEVER used for real-time risk judgement —
//! see docs/risk-control-system-design.md §5.5 (HTT / RDDT production bugs).

use crate::db::Database;
use crate::models::risk::*;
use crate::models::ExchangeRates;
use crate::services::data_quality;
use crate::services::exchange_rate_service::convert_currency;
use crate::services::quote_service::is_cash_symbol;
use crate::services::risk_engine::{
    evaluate_all, severity_rank, RULE_CONCENTRATION, RULE_MAX_DRAWDOWN, RULE_STALE_DATA,
    RULE_STOP_LOSS,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use std::collections::{HashMap, HashSet};

const HWM_SCOPE: &str = "GLOBAL";
/// 风控首版固定以 USD 为基准，与仪表盘/统计口径一致。
pub const BASE_CURRENCY: &str = "USD";

pub fn now_utc_str() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub(crate) fn load_rates(conn: &Connection) -> ExchangeRates {
    conn.query_row(
        "SELECT usd_cny, usd_hkd, cny_hkd, updated_at FROM cached_exchange_rates WHERE id = 1",
        [],
        |r| {
            Ok(ExchangeRates {
                usd_cny: r.get(0)?,
                usd_hkd: r.get(1)?,
                cny_hkd: r.get(2)?,
                updated_at: r.get(3)?,
            })
        },
    )
    .unwrap_or_else(|_| ExchangeRates {
        usd_cny: 7.2,
        usd_hkd: 7.8,
        cny_hkd: 7.8 / 7.2,
        updated_at: now_utc_str(),
    })
}

// ────────────────────────────────────────────────────────────────
// Rule configuration
// ────────────────────────────────────────────────────────────────

fn row_to_rule(r: &rusqlite::Row<'_>) -> rusqlite::Result<RiskRuleConfig> {
    Ok(RiskRuleConfig {
        id: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        severity: r.get(3)?,
        enabled: r.get::<_, i32>(4)? != 0,
        threshold: r.get(5)?,
        params: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

const RULE_SELECT: &str = "SELECT id, name, description, severity, enabled, threshold, params, updated_at FROM risk_rules";

pub fn list_rules(db: &Database) -> Result<Vec<RiskRuleConfig>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(&format!("{} ORDER BY id", RULE_SELECT))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_rule)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

fn load_rules(conn: &Connection) -> Result<HashMap<String, RiskRuleConfig>, String> {
    let mut stmt = conn.prepare(RULE_SELECT).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_rule)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|r| (r.id.clone(), r)).collect())
}

pub fn update_rule(
    db: &Database,
    id: &str,
    enabled: Option<bool>,
    threshold: Option<f64>,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let current: Option<(i32, Option<f64>)> = conn
        .query_row(
            "SELECT enabled, threshold FROM risk_rules WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let (cur_enabled, cur_threshold) = current.ok_or_else(|| format!("未知规则: {}", id))?;
    let new_enabled = enabled
        .map(|b| if b { 1 } else { 0 })
        .unwrap_or(cur_enabled);
    let new_threshold = threshold.or(cur_threshold);
    conn.execute(
        "UPDATE risk_rules SET enabled = ?2, threshold = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, new_enabled, new_threshold, now_utc_str()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ────────────────────────────────────────────────────────────────
// Real-time context
// ────────────────────────────────────────────────────────────────

struct RawHolding {
    account_id: String,
    symbol: String,
    name: String,
    market: String,
    shares: f64,
    avg_cost: f64,
    currency: String,
}

pub(crate) fn quote_age_secs(updated_at: &str) -> i64 {
    DateTime::parse_from_rfc3339(updated_at)
        .map(|dt| (Utc::now() - dt.with_timezone(&Utc)).num_seconds().max(0))
        .unwrap_or(i64::MAX / 4)
}

/// Build the live risk context. Returns an error only on genuine DB failure;
/// missing quotes are represented as data-quality findings, never as price 0.
pub fn build_context(db: &Database) -> Result<RiskContext, String> {
    build_context_with_options(db, true)
}

/// `persist_nav = false` is used by P4 stress tests: build an identical context
/// but do NOT advance the persisted high-water mark. A what-if preview must
/// never change real risk state.
pub fn build_context_with_options(
    db: &Database,
    persist_nav: bool,
) -> Result<RiskContext, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let rules = load_rules(&conn)?;
    let rates = load_rates(&conn);
    let stale_mins = rules
        .get(RULE_STALE_DATA)
        .and_then(|r| r.threshold)
        .unwrap_or(1440.0);
    let stale_secs = (stale_mins * 60.0) as i64;

    let holdings: Vec<RawHolding> = {
        let mut stmt = conn
            .prepare(
                "SELECT account_id, symbol, name, market, shares, avg_cost, currency
                 FROM holdings WHERE shares != 0",
            )
            .map_err(|e| e.to_string())?;
        let mapped = stmt
            .query_map([], |r| {
                Ok(RawHolding {
                    account_id: r.get(0)?,
                    symbol: r.get(1)?,
                    name: r.get(2)?,
                    market: r.get(3)?,
                    shares: r.get(4)?,
                    avg_cost: r.get(5)?,
                    currency: r.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let rows = mapped
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };

    // Symbol case differs between holdings and cached_quotes (e.g. "TCOM" vs
    // "tcom"), so key the quote map case-insensitively and keep the newest row.
    let mut quotes: HashMap<String, (f64, String)> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT symbol, current_price, updated_at FROM cached_quotes")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (sym, price, ts) in rows {
            let key = sym.to_lowercase();
            let fresh = quotes
                .get(&key)
                .map(|(_, old_ts)| ts > *old_ts)
                .unwrap_or(true);
            if fresh {
                quotes.insert(key, (price, ts));
            }
        }
    }

    let mut positions = Vec::new();
    let mut cash_value_base = 0.0f64;
    let mut total_assets_base = 0.0f64;
    let mut missing_symbols = Vec::new();
    let mut stale_symbols = Vec::new();
    let mut oldest_age = 0i64;

    for h in holdings {
        if is_cash_symbol(&h.symbol) {
            // 现金持仓 price 恒为 1，shares 即余额（可为负＝融资/透支）。
            // 必须计入总资产：否则买入会让 V_t 凭空增加，TWR 会把买入误判为收益。
            let native = h.shares; // shares × 1.0
            cash_value_base += convert_currency(native, &h.currency, BASE_CURRENCY, &rates);
            total_assets_base += convert_currency(native, &h.currency, BASE_CURRENCY, &rates);
            continue;
        }

        let q = quotes.get(&h.symbol.to_lowercase());
        let (price, has_quote, age) = match q {
            Some((p, ts)) if *p > 0.0 => (*p, true, quote_age_secs(ts)),
            _ => (h.avg_cost, false, i64::MAX / 4),
        };

        if has_quote {
            oldest_age = oldest_age.max(age);
            if age > stale_secs {
                stale_symbols.push(h.symbol.clone());
            }
        } else {
            missing_symbols.push(h.symbol.clone());
        }

        // 无行情时用成本兜底估算市值，避免总资产被低估（但不参与止损/集中度判定）。
        let mv_base = convert_currency(h.shares * price, &h.currency, BASE_CURRENCY, &rates);
        total_assets_base += mv_base;
        let pnl_percent = if has_quote && h.avg_cost > 0.0 {
            (price - h.avg_cost) / h.avg_cost * 100.0
        } else {
            0.0
        };

        positions.push(RiskPosition {
            symbol: h.symbol,
            name: h.name,
            market: h.market,
            account_id: h.account_id,
            currency: h.currency,
            shares: h.shares,
            avg_cost: h.avg_cost,
            price,
            market_value_base: mv_base,
            pnl_percent,
            has_quote,
            quote_age_secs: age,
            is_cash: false,
        });
    }

    let read_only_hwm = get_hwm_with_conn(&conn)?;
    drop(conn);

    // NAV state is persisted (HWM only ever increases) and must be computed
    // after the connection used above is released — except in what-if mode.
    let hwm = if persist_nav {
        update_nav_state(db, total_assets_base)?
    } else {
        read_only_hwm
    };

    Ok(RiskContext {
        as_of: now_utc_str(),
        base_currency: BASE_CURRENCY.to_string(),
        positions,
        cash_value_base,
        total_assets_base,
        unit_nav: hwm.current_unit_nav,
        peak_unit_nav: hwm.peak_unit_nav,
        rules,
        freshness: QuoteFreshness {
            has_quotes: !quotes.is_empty(),
            stale_symbols,
            missing_symbols,
            oldest_quote_age_secs: oldest_age,
            stale_threshold_secs: stale_secs,
        },
    })
}

// ────────────────────────────────────────────────────────────────
// Unit NAV + HWM (TWR-based, immune to deposits/withdrawals)
// ────────────────────────────────────────────────────────────────

fn row_to_hwm(r: &rusqlite::Row<'_>) -> rusqlite::Result<HwmState> {
    Ok(HwmState {
        scope_key: r.get(0)?,
        peak_unit_nav: r.get(1)?,
        peak_date: r.get(2)?,
        current_unit_nav: r.get(3)?,
        base_value: r.get(4)?,
        base_date: r.get(5)?,
        net_external_flow: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

const HWM_SELECT: &str = "SELECT scope_key, peak_unit_nav, peak_date, current_unit_nav, base_value, base_date, net_external_flow, updated_at FROM portfolio_hwm";

pub fn get_hwm(db: &Database) -> Result<HwmState, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    get_hwm_with_conn(&conn)
}

fn get_hwm_with_conn(conn: &Connection) -> Result<HwmState, String> {
    conn.query_row(
        &format!("{} WHERE scope_key = ?1", HWM_SELECT),
        params![HWM_SCOPE],
        row_to_hwm,
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "HWM 尚未初始化".to_string())
    .or_else(|_| {
        // Lazy init even if no evaluation has run yet.
        let now = now_utc_str();
        conn.execute(
            "INSERT OR IGNORE INTO portfolio_hwm
             (scope_key, peak_unit_nav, peak_date, current_unit_nav, base_value, base_date, net_external_flow, updated_at)
             VALUES (?1, 1.0, ?2, 1.0, 0.0, ?2, 0.0, ?2)",
            params![HWM_SCOPE, now],
        )
        .map_err(|e| e.to_string())?;
        conn.query_row(
            &format!("{} WHERE scope_key = ?1", HWM_SELECT),
            params![HWM_SCOPE],
            row_to_hwm,
        )
        .map_err(|e| e.to_string())
    })
}

/// Recompute the unit NAV and persist the (monotonic) high-water mark.
///
/// 口径说明（首版）：
/// `u(t) = (V(t) − ΣF) / V(base)`，其中 ΣF 是启用日后、尚未计入基准的
/// 净外部现金流（已折算 USD）。V(t) 含现金持仓，故内部买卖自动抵消；
/// F 由 `external_cash_flows` 提供（方案 A），故入金不掩盖回撤、出金不误触熔断。
/// 历史 `daily_portfolio_values` 因口径不含现金，**不参与熔断判定**。
pub fn update_nav_state(db: &Database, total_assets_base: f64) -> Result<HwmState, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let now = now_utc_str();
    let mut state = get_hwm_with_conn(&conn)?;

    // 首次锚定：以当前实时总资产为基准，u0 = 1.0。
    if state.base_value <= 0.0 {
        if total_assets_base <= 0.0 {
            return Ok(state);
        }
        state.base_value = total_assets_base;
        state.base_date = now.clone();
        conn.execute(
            "UPDATE portfolio_hwm SET base_value = ?2, base_date = ?3 WHERE scope_key = ?1",
            params![HWM_SCOPE, state.base_value, state.base_date],
        )
        .map_err(|e| e.to_string())?;
    }

    let net_flow: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(amount_base), 0.0) FROM external_cash_flows WHERE counted_in_base = 0",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;

    let mut unit_nav = (total_assets_base - net_flow) / state.base_value;
    if !unit_nav.is_finite() || unit_nav <= 0.0 {
        unit_nav = 1.0;
    }
    // 防御：行情异常导致的 >95% 单日跳变不落库（防止 HWM 被脏数据锁死）。
    // 阈值统一由 data_quality 维护，避免两处常量漂移。
    if data_quality::validate_unit_nav(state.current_unit_nav, unit_nav).is_err() {
        unit_nav = state.current_unit_nav;
    }

    if unit_nav > state.peak_unit_nav {
        state.peak_unit_nav = unit_nav;
        state.peak_date = now.clone();
    }
    state.current_unit_nav = unit_nav;
    state.net_external_flow = net_flow;
    state.updated_at = now.clone();

    conn.execute(
        "UPDATE portfolio_hwm
         SET peak_unit_nav = ?2, peak_date = ?3, current_unit_nav = ?4,
             net_external_flow = ?5, updated_at = ?6
         WHERE scope_key = ?1",
        params![
            HWM_SCOPE,
            state.peak_unit_nav,
            state.peak_date,
            state.current_unit_nav,
            state.net_external_flow,
            state.updated_at
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(state)
}

/// Reset the NAV anchor. Left deliberately explicit + audited: a user must not
/// be able to quietly "reset the water mark" whenever a drawdown looks bad.
pub fn recalibrate_hwm(db: &Database, reason: &str) -> Result<HwmState, String> {
    if reason.trim().is_empty() {
        return Err("校准 HWM 必须填写原因".to_string());
    }
    let ctx = build_context(db)?;
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let now = now_utc_str();
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

    // 已有现金流的效果已包含在新的 base_value 里，避免二次扣除。
    tx.execute(
        "UPDATE external_cash_flows SET counted_in_base = 1 WHERE counted_in_base = 0",
        [],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "UPDATE portfolio_hwm
         SET base_value = ?2, base_date = ?3, peak_unit_nav = 1.0, peak_date = ?3,
             current_unit_nav = 1.0, net_external_flow = 0.0, updated_at = ?3
         WHERE scope_key = ?1",
        params![HWM_SCOPE, ctx.total_assets_base, now],
    )
    .map_err(|e| e.to_string())?;
    // NOTE: ?5 is intentionally reused for first/last/resolved_at. The number of
    // params! entries must equal the LARGEST placeholder index (5 here), not the
    // number of columns. Do not append another `now` — see tests/check_sql_bindings.py.
    tx.execute(
        "INSERT INTO risk_events
         (id, rule_id, symbol, account_id, severity, status, metric_value, threshold,
          message, context, first_triggered_at, last_triggered_at, resolved_at)
         VALUES (?1, 'HWM_RECALIBRATE', NULL, NULL, 'info', 'resolved', ?2, NULL,
                 ?3, ?4, ?5, ?5, ?5)",
        params![
            uuid::Uuid::new_v4().to_string(),
            ctx.total_assets_base,
            format!("HWM 校准：{}", reason.trim()),
            json!({ "total_assets_base": ctx.total_assets_base }).to_string(),
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    drop(conn);

    get_hwm(db)
}

// ────────────────────────────────────────────────────────────────
// Event de-duplication state machine
// ────────────────────────────────────────────────────────────────

fn row_to_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<RiskEvent> {
    Ok(RiskEvent {
        id: r.get(0)?,
        rule_id: r.get(1)?,
        symbol: r.get(2)?,
        account_id: r.get(3)?,
        severity: r.get(4)?,
        status: r.get(5)?,
        metric_value: r.get(6)?,
        threshold: r.get(7)?,
        message: r.get(8)?,
        context: r.get(9)?,
        first_triggered_at: r.get(10)?,
        last_triggered_at: r.get(11)?,
        resolved_at: r.get(12)?,
    })
}

const EVENT_SELECT: &str = "SELECT id, rule_id, symbol, account_id, severity, status, metric_value, threshold, message, context, first_triggered_at, last_triggered_at, resolved_at FROM risk_events";

fn sync_events(conn: &Connection, findings: &[RiskFinding]) -> Result<Vec<RiskEvent>, String> {
    let now = now_utc_str();

    // (rule_id, symbol-or-*) -> (id, status, severity)
    let mut existing: HashMap<(String, String), (String, String, String)> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT id, rule_id, IFNULL(symbol, '*'), status, severity
                 FROM risk_events WHERE status IN ('active', 'acknowledged')",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (id, rule_id, sym, status, severity) in rows {
            existing.insert((rule_id, sym), (id, status, severity));
        }
    }

    let mut seen: HashSet<(String, String)> = HashSet::new();
    for f in findings {
        let sym = f.symbol.clone().unwrap_or_else(|| "*".to_string());
        let key = (f.rule_id.clone(), sym.clone());
        seen.insert(key.clone());
        let context = f.context.to_string();

        if let Some((id, status, old_sev)) = existing.get(&key) {
            conn.execute(
                "UPDATE risk_events
                 SET last_triggered_at = ?2, metric_value = ?3, threshold = ?4,
                     message = ?5, context = ?6, severity = ?7
                 WHERE id = ?1",
                params![id, now, f.metric, f.threshold, f.message, context, f.severity],
            )
            .map_err(|e| e.to_string())?;
            // 严重度升级时重新提醒（即使此前已确认）
            if severity_rank(&f.severity) > severity_rank(old_sev) && status == STATUS_ACKNOWLEDGED {
                conn.execute(
                    "UPDATE risk_events SET status = 'active' WHERE id = ?1",
                    params![id],
                )
                .map_err(|e| e.to_string())?;
            }
        } else {
            // NOTE: ?9 is intentionally reused for first/last_triggered_at.
            // params! must contain exactly 9 entries (the largest placeholder index).
            conn.execute(
                "INSERT INTO risk_events
                 (id, rule_id, symbol, account_id, severity, status, metric_value, threshold,
                  message, context, first_triggered_at, last_triggered_at, resolved_at)
                 VALUES (?1, ?2, ?3, NULL, ?4, 'active', ?5, ?6, ?7, ?8, ?9, ?9, NULL)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    f.rule_id,
                    f.symbol,
                    f.severity,
                    f.metric,
                    f.threshold,
                    f.message,
                    context,
                    now
                ],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    for (key, (id, _status, _sev)) in &existing {
        if !seen.contains(key) {
            conn.execute(
                "UPDATE risk_events SET status = 'resolved', resolved_at = ?2 WHERE id = ?1",
                params![id, now],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    let mut stmt = conn
        .prepare(&format!(
            "{} WHERE status IN ('active', 'acknowledged')
             ORDER BY CASE severity WHEN 'block' THEN 4 WHEN 'critical' THEN 3
                                   WHEN 'warning' THEN 2 ELSE 1 END DESC,
                      last_triggered_at DESC",
            EVENT_SELECT
        ))
        .map_err(|e| e.to_string())?;
    let events = stmt
        .query_map([], row_to_event)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(events)
}

pub fn acknowledge_event(db: &Database, id: &str) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE risk_events SET status = 'acknowledged' WHERE id = ?1 AND status = 'active'",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ────────────────────────────────────────────────────────────────
// Evaluation + dashboard
// ────────────────────────────────────────────────────────────────

pub fn evaluate_risk(db: &Database) -> Result<Vec<RiskFinding>, String> {
    let ctx = build_context(db)?;
    let findings = evaluate_all(&ctx);
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    sync_events(&conn, &findings)?;
    Ok(findings)
}

pub fn get_dashboard(db: &Database) -> Result<RiskDashboard, String> {
    let ctx = build_context(db)?;
    let findings = evaluate_all(&ctx);
    let events = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        sync_events(&conn, &findings)?
    };
    let rules = list_rules(db)?;

    let status = if findings.iter().any(|f| f.severity == SEV_BLOCK) {
        "blocked"
    } else if findings
        .iter()
        .any(|f| f.severity == SEV_CRITICAL || f.severity == SEV_WARNING)
    {
        "warning"
    } else {
        "normal"
    };

    let drawdown = if ctx.peak_unit_nav > 0.0 {
        (ctx.unit_nav - ctx.peak_unit_nav) / ctx.peak_unit_nav * 100.0
    } else {
        0.0
    };

    let mut max_weight = 0.0f64;
    let mut max_weight_symbol = String::new();
    if ctx.total_assets_base > 0.0 {
        for p in ctx
            .positions
            .iter()
            .filter(|p| !p.is_cash && p.shares > 0.0 && p.has_quote && p.price > 0.0)
        {
            let w = p.market_value_base / ctx.total_assets_base * 100.0;
            if w > max_weight {
                max_weight = w;
                max_weight_symbol = p.symbol.clone();
            }
        }
    }

    let threshold_of = |id: &str, default: f64| -> f64 {
        ctx.rules
            .get(id)
            .and_then(|r| r.threshold)
            .unwrap_or(default)
    };

    let hwm = get_hwm(db)?;
    let hwm_note = format!(
        "HWM 自 {} 起算（单位净值法，剔除出入金）；历史快照口径不含现金，不参与熔断。",
        hwm.base_date.get(..10).unwrap_or(&hwm.base_date)
    );

    Ok(RiskDashboard {
        status: status.to_string(),
        as_of: ctx.as_of,
        base_currency: ctx.base_currency,
        total_assets: ctx.total_assets_base,
        cash_value: ctx.cash_value_base,
        unit_nav: ctx.unit_nav,
        peak_unit_nav: ctx.peak_unit_nav,
        current_drawdown: drawdown,
        drawdown_threshold: threshold_of(RULE_MAX_DRAWDOWN, -5.0),
        stop_loss_count: findings.iter().filter(|f| f.rule_id == RULE_STOP_LOSS).count(),
        stop_loss_threshold: threshold_of(RULE_STOP_LOSS, -20.0),
        max_weight,
        max_weight_symbol,
        concentration_threshold: threshold_of(RULE_CONCENTRATION, 15.0),
        net_external_flow: hwm.net_external_flow,
        hwm_base_date: hwm.base_date,
        hwm_note,
        position_count: ctx.positions.iter().filter(|p| !p.is_cash).count(),
        quote_freshness: ctx.freshness,
        findings,
        events,
        rules,
    })
}
