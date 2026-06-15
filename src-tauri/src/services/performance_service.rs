use crate::db::Database;
use crate::services::http_client;
use chrono::Datelike;

const RISK_FREE_RATE: f64 = 0.045; // 4.5% US 10-year treasury default
const TRADING_DAYS_PER_YEAR: f64 = 252.0;
const CACHE_COVERAGE_THRESHOLD: f64 = 0.5; // require 50% of expected days in cache to skip re-fetch
use crate::models::performance::{
    annualise_return, AttributionItem, BenchmarkDataPoint,
    DrawdownAnalysis, DrawdownPoint, HoldingPerformance, MonthlyReturn, PerformanceSummary,
    ReturnAttribution, ReturnDataPoint, RiskMetrics,
};
use chrono::NaiveDate;

// ─────────────────────────────────────────────────────────────────────────────
// Internal DB helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Optional filters for narrowing performance analysis to a specific market
/// or account.
#[derive(Debug, Clone, Default)]
pub struct PerformanceFilter {
    pub market: Option<String>,
    pub account_id: Option<String>,
}

impl PerformanceFilter {
    pub fn is_active(&self) -> bool {
        self.market.is_some() || self.account_id.is_some()
    }

    /// Append optional WHERE clauses for market/account_id and push
    /// corresponding parameter values. Returns the number of params added.
    fn append_where_clauses(
        &self,
        sql: &mut String,
        params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    ) {
        if let Some(ref market) = self.market {
            sql.push_str(&format!(" AND market = ?{}", params.len() + 1));
            params.push(Box::new(market.clone()));
        }
        if let Some(ref account_id) = self.account_id {
            sql.push_str(&format!(" AND account_id = ?{}", params.len() + 1));
            params.push(Box::new(account_id.clone()));
        }
    }
}

/// Fetch daily portfolio values (total_value, total_cost, daily_pnl,
/// cumulative_pnl) for the date range.
fn fetch_daily_values(
    db: &Database,
    start: NaiveDate,
    end: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<Vec<(NaiveDate, f64, f64, f64, f64)>, String> {
    if filter.is_active() {
        return fetch_filtered_daily_values(db, start, end, filter);
    }
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = start.format("%Y-%m-%d").to_string();
    let end_str = end.format("%Y-%m-%d").to_string();

    let mut stmt = conn
        .prepare(
            "SELECT date, total_value, total_cost, daily_pnl, cumulative_pnl
             FROM daily_portfolio_values
             WHERE date BETWEEN ?1 AND ?2
             ORDER BY date ASC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(rusqlite::params![start_str, end_str], |row| {
            let date_str: String = row.get(0)?;
            let value: f64 = row.get(1)?;
            let cost: f64 = row.get(2)?;
            let dpnl: f64 = row.get(3)?;
            let cpnl: f64 = row.get(4)?;
            Ok((date_str, value, cost, dpnl, cpnl))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    rows.into_iter()
        .map(|(ds, v, c, d, cp)| {
            let date = NaiveDate::parse_from_str(&ds, "%Y-%m-%d")
                .map_err(|e| format!("bad date '{}': {}", ds, e))?;
            Ok((date, v, c, d, cp))
        })
        .collect()
}

/// Fetch daily values from `daily_holding_snapshots` aggregated by date,
/// filtered by market and/or account_id. Derives daily_pnl from consecutive
/// day value differences.  Cost and cumulative_pnl are approximated from
/// the aggregated values (snapshots only store market_value).
fn fetch_filtered_daily_values(
    db: &Database,
    start: NaiveDate,
    end: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<Vec<(NaiveDate, f64, f64, f64, f64)>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = start.format("%Y-%m-%d").to_string();
    let end_str = end.format("%Y-%m-%d").to_string();

    let mut sql = String::from(
        "SELECT date, SUM(market_value) as total_value
         FROM daily_holding_snapshots
         WHERE date BETWEEN ?1 AND ?2",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(start_str),
        Box::new(end_str),
    ];

    filter.append_where_clauses(&mut sql, &mut params);

    sql.push_str(" GROUP BY date ORDER BY date ASC");

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            let date_str: String = row.get(0)?;
            let value: f64 = row.get(1)?;
            Ok((date_str, value))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut result = Vec::with_capacity(rows.len());
    let mut prev_value: Option<f64> = None;
    let mut cum_pnl = 0.0_f64;
    for (ds, v) in rows {
        let date = NaiveDate::parse_from_str(&ds, "%Y-%m-%d")
            .map_err(|e| format!("bad date '{}': {}", ds, e))?;
        let dpnl = prev_value.map(|pv| v - pv).unwrap_or(0.0);
        cum_pnl += dpnl;
        // Snapshots don't store cost basis; use value as cost approximation
        // so that return = (value - cost) / cost ≈ 0 for filtered view.
        // TODO: join with transactions table for proper cost in filtered mode.
        result.push((date, v, v, dpnl, cum_pnl));
        prev_value = Some(v);
    }
    Ok(result)
}

/// Fetch the portfolio value on the latest day strictly before `date`.
/// Used as the baseline for cumulative-return curves so that the first
/// visible day already shows a non-zero return.
fn fetch_previous_day_value(db: &Database, date: NaiveDate, filter: &PerformanceFilter) -> Result<Option<f64>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let date_str = date.format("%Y-%m-%d").to_string();
    if filter.is_active() {
        let mut sql = String::from(
            "SELECT SUM(market_value) FROM daily_holding_snapshots WHERE date = (SELECT MAX(date) FROM daily_holding_snapshots WHERE date < ?1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(date_str.clone())];
        filter.append_where_clauses(&mut sql, &mut params);
        sql.push(')');
        // Apply same filters to outer WHERE
        filter.append_where_clauses(&mut sql, &mut params);
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let result = conn
            .query_row(&sql, param_refs.as_slice(), |row| row.get::<_, f64>(0))
            .ok();
        return Ok(result);
    }
    let result = conn
        .query_row(
            "SELECT total_value FROM daily_portfolio_values WHERE date < ?1 ORDER BY date DESC LIMIT 1",
            rusqlite::params![date_str],
            |row| row.get::<_, f64>(0),
        )
        .ok();
    Ok(result)
}

// ─────────────────────────────────────────────────────────────────────────────
// Core calculations
// ─────────────────────────────────────────────────────────────────────────────

/// Build a Vec<ReturnDataPoint> from the daily portfolio values.
/// Each row is (date, total_value, total_cost, daily_pnl, cumulative_pnl).
///
/// When `inception_value` is provided, cumulative returns are calculated
/// relative to this value (the portfolio value at creation) instead of the
/// first element in `daily_values`.
///
/// **Return calculation:** cumulative_return uses `cumulative_pnl / total_cost`
/// so that returns reflect actual profit/loss relative to invested capital,
/// NOT market-value changes that conflate investment inflows with gains.
///
/// **Note:** The returned `daily_return` and `cumulative_return` fields are
/// already in **percentage** form (e.g. 1.5 means 1.5%). Callers that need
/// decimal returns (e.g. for volatility or Sharpe calculations) must divide
/// by 100.
pub fn build_return_series(
    daily_values: &[(NaiveDate, f64, f64, f64, f64)],
    inception_value: Option<f64>,
) -> Vec<ReturnDataPoint> {
    if daily_values.is_empty() {
        return vec![];
    }

    // Legacy inception_value is kept for backward-compat but no longer drives
    // the cumulative-return formula (cost-based PnL is used instead).
    let _start_value = inception_value.unwrap_or(daily_values[0].1);
    let mut prev_value = daily_values[0].1;
    let mut result = Vec::with_capacity(daily_values.len());

    for (date, value, cost, dpnl, cpnl) in daily_values {
        // Daily return: price movement / previous day's value
        let daily_return = if prev_value > 0.0 {
            (value - prev_value) / prev_value
        } else {
            0.0
        };
        // Cumulative return: actual PnL relative to cost basis.
        // This correctly handles additional capital inflows (new positions,
        // deposits) because cpnl tracks real profit/loss, not value change.
        let cumulative_return = if *cost > 0.0 {
            *cpnl / *cost
        } else {
            0.0
        };
        result.push(ReturnDataPoint {
            date: date.format("%Y-%m-%d").to_string(),
            cumulative_return: cumulative_return * 100.0,
            daily_return: daily_return * 100.0,
            portfolio_value: *value,
            daily_pnl: *dpnl,
        });
        prev_value = *value;
    }
    result
}

/// Calculate maximum drawdown from a return series.
pub fn calculate_max_drawdown(return_series: &[ReturnDataPoint]) -> DrawdownAnalysis {
    if return_series.is_empty() {
        return DrawdownAnalysis {
            max_drawdown: 0.0,
            peak_date: String::new(),
            trough_date: String::new(),
            recovery_date: None,
            drawdown_duration: 0,
            recovery_duration: None,
            drawdown_series: vec![],
        };
    }

    let values: Vec<f64> = return_series.iter().map(|r| r.portfolio_value).collect();
    let dates: Vec<&str> = return_series.iter().map(|r| r.date.as_str()).collect();

    let mut peak = values[0];
    let mut peak_idx = 0usize;
    let mut max_drawdown = 0.0f64;
    let mut md_peak_idx = 0usize;
    let mut md_trough_idx = 0usize;

    let mut drawdown_series = Vec::with_capacity(values.len());

    for (i, &v) in values.iter().enumerate() {
        if v > peak {
            peak = v;
            peak_idx = i;
        }
        let dd = if peak > 0.0 { (v - peak) / peak } else { 0.0 };
        drawdown_series.push(DrawdownPoint {
            date: dates[i].to_string(),
            drawdown: dd * 100.0,
        });
        if dd < max_drawdown {
            max_drawdown = dd;
            md_peak_idx = peak_idx;
            md_trough_idx = i;
        }
    }

    // Find recovery date: first date after trough where value >= peak at trough time
    let peak_value_at_md = values[md_peak_idx];
    let recovery_idx = values[md_trough_idx..]
        .iter()
        .position(|&v| v >= peak_value_at_md)
        .map(|offset| md_trough_idx + offset);

    let peak_date_str = dates[md_peak_idx].to_string();
    let trough_date_str = dates[md_trough_idx].to_string();

    let drawdown_duration = if let (Ok(pd), Ok(td)) = (
        NaiveDate::parse_from_str(&peak_date_str, "%Y-%m-%d"),
        NaiveDate::parse_from_str(&trough_date_str, "%Y-%m-%d"),
    ) {
        (td - pd).num_days()
    } else {
        0
    };

    let recovery_date = recovery_idx.map(|ri| dates[ri].to_string());
    let recovery_duration = recovery_date.as_deref().and_then(|rd| {
        let td = NaiveDate::parse_from_str(&trough_date_str, "%Y-%m-%d").ok()?;
        let rdate = NaiveDate::parse_from_str(rd, "%Y-%m-%d").ok()?;
        Some((rdate - td).num_days())
    });

    DrawdownAnalysis {
        max_drawdown: max_drawdown * 100.0,
        peak_date: peak_date_str,
        trough_date: trough_date_str,
        recovery_date,
        drawdown_duration,
        recovery_duration,
        drawdown_series,
    }
}

/// Calculate annualised volatility from daily return percentages.
pub fn calculate_volatility(daily_returns: &[f64]) -> (f64, f64) {
    let n = daily_returns.len();
    if n < 2 {
        return (0.0, 0.0);
    }
    let mean = daily_returns.iter().sum::<f64>() / n as f64;
    let variance = daily_returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
    let daily_vol = variance.sqrt();
    let annualised_vol = daily_vol * TRADING_DAYS_PER_YEAR.sqrt();
    (daily_vol, annualised_vol)
}

/// Calculate Sharpe ratio.
pub fn calculate_sharpe(annualised_return: f64, risk_free_rate: f64, annualised_vol: f64) -> f64 {
    if annualised_vol == 0.0 {
        return 0.0;
    }
    (annualised_return - risk_free_rate) / annualised_vol
}

// ─────────────────────────────────────────────────────────────────────────────
// Public service functions called from commands
// ─────────────────────────────────────────────────────────────────────────────

pub fn get_return_series(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<Vec<ReturnDataPoint>, String> {
    let daily = fetch_daily_values(db, start_date, end_date, filter)?;
    let base_value = fetch_previous_day_value(db, start_date, filter)?;
    Ok(build_return_series(&daily, base_value))
}

pub fn get_performance_summary(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<PerformanceSummary, String> {
    let daily = fetch_daily_values(db, start_date, end_date, filter)?;
    if daily.is_empty() {
        return Ok(PerformanceSummary {
            start_date: start_date.format("%Y-%m-%d").to_string(),
            end_date: end_date.format("%Y-%m-%d").to_string(),
            start_value: 0.0,
            end_value: 0.0,
            total_return: 0.0,
            annualized_return: 0.0,
            total_pnl: 0.0,
            max_drawdown: 0.0,
            volatility: 0.0,
            sharpe_ratio: 0.0,
            return_series: vec![],
        });
    }

    // Use the portfolio value on the day *before* the range as the baseline,
    // exactly the same as get_return_series() does for the cumulative-return
    // chart.  Falling back to the first in-range value keeps the maths safe
    // when there is no prior snapshot.
    let base_value = fetch_previous_day_value(db, start_date, filter)?;
    let start_value = base_value.unwrap_or(daily[0].1);
    let end_value = daily.last().unwrap().1;

    // Use actual cumulative PnL from the data, NOT (end_value - start_value)
    // which conflates investment inflows with trading gains.
    let total_pnl = daily.last().map(|(_, _, _, _, cpnl)| *cpnl).unwrap_or(0.0);

    // Build the return series FIRST, then derive total_return from its last
    // cumulative_return.  This guarantees the summary card value matches the
    // cumulative-return chart exactly (avoids tiny floating-point divergence
    // that can arise when two independent computations of the same formula
    // are compiled into different machine-code sequences).
    let return_series = build_return_series(&daily, base_value);
    let total_return_pct = return_series
        .last()
        .map(|r| r.cumulative_return)
        .unwrap_or(0.0);
    let total_return = total_return_pct / 100.0; // decimal form for annualisation

    let days = (end_date - start_date).num_days();
    let annualised = annualise_return(total_return, days);

    let dd_analysis = calculate_max_drawdown(&return_series);

    // daily_return values from build_return_series are already in percentage
    // form (e.g. 1.5 means 1.5%), so convert to decimal for volatility/Sharpe.
    let daily_returns: Vec<f64> = return_series.iter().map(|r| r.daily_return / 100.0).collect();
    let (_daily_vol, ann_vol) = calculate_volatility(&daily_returns);
    let sharpe = calculate_sharpe(annualised, RISK_FREE_RATE, ann_vol);

    Ok(PerformanceSummary {
        start_date: start_date.format("%Y-%m-%d").to_string(),
        end_date: end_date.format("%Y-%m-%d").to_string(),
        start_value,
        end_value,
        total_return: total_return_pct,
        annualized_return: annualised * 100.0,
        total_pnl,
        max_drawdown: dd_analysis.max_drawdown,
        volatility: ann_vol * 100.0,
        sharpe_ratio: sharpe,
        return_series,
    })
}

pub fn get_risk_metrics(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<RiskMetrics, String> {
    let daily = fetch_daily_values(db, start_date, end_date, filter)?;
    if daily.is_empty() {
        return Ok(RiskMetrics {
            daily_volatility: 0.0,
            annualized_volatility: 0.0,
            sharpe_ratio: 0.0,
            risk_free_rate: 4.5,
            max_drawdown: 0.0,
            calmar_ratio: 0.0,
        });
    }

    let base_value = fetch_previous_day_value(db, start_date, filter)?;

    let return_series = build_return_series(&daily, base_value);
    let total_return_pct = return_series
        .last()
        .map(|r| r.cumulative_return)
        .unwrap_or(0.0);
    let total_return = total_return_pct / 100.0;

    let days = (end_date - start_date).num_days();
    let annualised = annualise_return(total_return, days);

    // daily_return values from build_return_series are already in percentage
    // form (e.g. 1.5 means 1.5%), so convert to decimal for volatility/Sharpe.
    let daily_returns: Vec<f64> = return_series.iter().map(|r| r.daily_return / 100.0).collect();
    let (daily_vol, ann_vol) = calculate_volatility(&daily_returns);

    let sharpe = calculate_sharpe(annualised, RISK_FREE_RATE, ann_vol);

    let dd_analysis = calculate_max_drawdown(&return_series);
    let max_dd = dd_analysis.max_drawdown.abs() / 100.0;
    let calmar = if max_dd > 0.0 { annualised / max_dd } else { 0.0 };

    Ok(RiskMetrics {
        daily_volatility: daily_vol * 100.0,
        annualized_volatility: ann_vol * 100.0,
        sharpe_ratio: sharpe,
        risk_free_rate: RISK_FREE_RATE * 100.0,
        max_drawdown: dd_analysis.max_drawdown,
        calmar_ratio: calmar,
    })
}

pub fn get_return_attribution(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<ReturnAttribution, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = start_date.format("%Y-%m-%d").to_string();
    let end_str = end_date.format("%Y-%m-%d").to_string();

    // Get start and end snapshots aggregated by symbol
    // start_vals: symbol -> (market, category_name, market_value)
    let mut start_vals: std::collections::HashMap<String, (String, String, f64)> =
        std::collections::HashMap::new();
    // end_vals: symbol -> (market, category_name, market_value)
    let mut end_vals: std::collections::HashMap<String, (String, String, f64)> =
        std::collections::HashMap::new();

    {
        // Build start query with filters applied to both subquery and outer query
        let mut sql = String::from(
            "SELECT symbol, market, COALESCE(category_name, '未分类'), SUM(market_value)
             FROM daily_holding_snapshots
             WHERE date = (
                 SELECT MAX(date) FROM daily_holding_snapshots WHERE date <= ?1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(start_str.clone())];
        filter.append_where_clauses(&mut sql, &mut params);
        sql.push(')');
        filter.append_where_clauses(&mut sql, &mut params);
        sql.push_str(" GROUP BY symbol");
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (sym, mkt, cat, val) in rows {
            let entry = start_vals.entry(sym).or_insert((mkt, cat, 0.0));
            entry.2 += val;
        }
    }

    {
        // Build end query with filters applied to both subquery and outer query
        // Also fetch market and category_name for proper attribution of newly opened positions
        let mut sql = String::from(
            "SELECT symbol, market, COALESCE(category_name, '未分类'), SUM(market_value)
             FROM daily_holding_snapshots
             WHERE date = (
                 SELECT MAX(date) FROM daily_holding_snapshots WHERE date <= ?1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(end_str.clone())];
        filter.append_where_clauses(&mut sql, &mut params);
        sql.push(')');
        filter.append_where_clauses(&mut sql, &mut params);
        sql.push_str(" GROUP BY symbol");
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, f64>(3)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (sym, mkt, cat, val) in rows {
            end_vals.entry(sym).or_insert((mkt, cat, 0.0)).2 += val;
        }
    }

    // Fetch net cash flows per symbol from transactions during the period.
    // BUY  → positive cash flow (money invested into the holding)
    // SELL → negative cash flow (money withdrawn from the holding)
    let mut net_cash_flows: std::collections::HashMap<String, f64> =
        std::collections::HashMap::new();
    {
        let actual_start_str = {
            // Use the actual snapshot start date (MAX(date) <= start_date)
            let mut sql = String::from(
                "SELECT MAX(date) FROM daily_holding_snapshots WHERE date <= ?1",
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(start_str.clone())];
            filter.append_where_clauses(&mut sql, &mut params);
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            conn.query_row(&sql, param_refs.as_slice(), |row| row.get::<_, Option<String>>(0))
                .map_err(|e| e.to_string())?
                .unwrap_or_else(|| start_str.clone())
        };
        let mut sql = String::from(
            "SELECT symbol, transaction_type, SUM(total_amount)
             FROM transactions
             WHERE DATE(traded_at) > ?1 AND DATE(traded_at) <= ?2",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(actual_start_str),
            Box::new(end_str.clone()),
        ];
        if let Some(ref account_id) = filter.account_id {
            sql.push_str(&format!(" AND account_id = ?{}", params.len() + 1));
            params.push(Box::new(account_id.clone()));
        }
        if let Some(ref market) = filter.market {
            sql.push_str(&format!(" AND market = ?{}", params.len() + 1));
            params.push(Box::new(market.clone()));
        }
        sql.push_str(" GROUP BY symbol, transaction_type");
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (sym, tx_type, amount) in rows {
            let flow = match tx_type.as_str() {
                "BUY" => amount,   // money invested
                "OPEN" => amount,  // opening a position (same as BUY for cash flow)
                "SELL" => -amount, // money withdrawn
                _ => 0.0,
            };
            *net_cash_flows.entry(sym).or_insert(0.0) += flow;
        }
    }

    // Fetch holding names for enriching by_holding items
    let holding_names: std::collections::HashMap<String, String> = {
        let mut name_stmt = conn
            .prepare("SELECT symbol, name FROM holdings")
            .map_err(|e| e.to_string())?;
        let rows = name_stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows.into_iter().collect()
    };

    // Fetch holding cost (avg_cost * shares) for symbols that have no
    // snapshot on the start date (opened before the analysis period).
    let holding_cost_map: std::collections::HashMap<String, f64> = {
        let mut map = std::collections::HashMap::new();
        let mut stmt = conn
            .prepare("SELECT symbol, avg_cost * shares FROM holdings")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (sym, cost) in rows {
            map.insert(sym, cost);
        }
        map
    };

    let all_symbols: std::collections::HashSet<String> = start_vals
        .keys()
        .chain(end_vals.keys())
        .cloned()
        .collect();

    let mut total_pnl = 0.0f64;
    let mut total_start_val = 0.0f64;
    let mut market_pnl: std::collections::HashMap<String, f64> =
        std::collections::HashMap::new();
    let mut category_pnl: std::collections::HashMap<String, f64> =
        std::collections::HashMap::new();
    let mut holding_pnl: std::collections::HashMap<String, (String, String, f64, f64, f64)> =
        std::collections::HashMap::new();

    for sym in &all_symbols {
        // Skip cash symbols ($CASH-CNY, $CASH-USD, $CASH-HKD) from attribution.
        // Cash holdings don't have entries in the transactions table, so their
        // PnL = ev − sv reflects the cash flow from buying/selling stocks, NOT
        // actual investment returns. Including them double-counts the trade
        // amounts that are already subtracted from individual stock PnLs.
        if crate::services::quote_service::is_cash_symbol(sym) {
            continue;
        }

        // Get market and category: prefer start_vals (actual at start_date),
        // fall back to end_vals (for newly opened positions after start_date).
        let (market, cat, sv) = start_vals
            .get(sym)
            .map(|(m, c, v)| (m.clone(), c.clone(), *v))
            .unwrap_or_else(|| {
                end_vals
                    .get(sym)
                    .map(|(m, c, _)| (m.clone(), c.clone(), 0.0))
                    .unwrap_or_else(|| ("Unknown".to_string(), "未分类".to_string(), 0.0))
            });
        let ev = end_vals.get(sym).map(|(_, _, v)| *v).unwrap_or(0.0);
        let cf = net_cash_flows.get(sym).copied().unwrap_or(0.0);

        // Compute PnL correctly for three cases:
        // 1. sv > 0:  Had position at start.  PnL = ev - sv - cf.
        // 2. sv = 0, cf > 0:  Bought during period.  Cost = cf, PnL = ev - cf.
        // 3. sv = 0, cf <= 0:  Position existed before period (no start snapshot
        //    because start_date is before first snapshot).  Use holdings.avg_cost
        //    as cost basis.  PnL = ev - cost_basis.
        let (actual_sv, pnl) = if sv > 0.0 {
            (sv, ev - sv - cf)
        } else if cf > 0.0 {
            // New position opened during the period
            (cf, ev - cf)
        } else {
            // Position existed before period but no start snapshot
            let cost = holding_cost_map.get(sym).copied().unwrap_or(ev);
            (cost, ev - cost)
        };

        total_pnl += pnl;
        total_start_val += actual_sv;
        *market_pnl.entry(market.clone()).or_insert(0.0) += pnl;
        *category_pnl.entry(cat.clone()).or_insert(0.0) += pnl;
        holding_pnl
            .entry(sym.clone())
            .and_modify(|e| {
                e.2 += pnl;
                e.3 += actual_sv;
                e.4 += ev;
            })
            .or_insert((market, cat, pnl, actual_sv, ev));
    }

    let make_items =
        |map: std::collections::HashMap<String, f64>| -> Vec<AttributionItem> {
            let mut items: Vec<AttributionItem> = map
                .into_iter()
                .map(|(name, pnl)| {
                    let contribution_percent = if total_pnl != 0.0 {
                        pnl / total_pnl.abs() * 100.0
                    } else {
                        0.0
                    };
                    let weight = if total_start_val != 0.0 {
                        pnl / total_start_val * 100.0
                    } else {
                        0.0
                    };
                    AttributionItem {
                        name,
                        pnl,
                        contribution_percent,
                        weight,
                    }
                })
                .collect();
            items.sort_by(|a, b| b.pnl.partial_cmp(&a.pnl).unwrap_or(std::cmp::Ordering::Equal));
            items
        };

    let market_label = |m: &str| match m {
        "US" => "🇺🇸 美股".to_string(),
        "CN" => "🇨🇳 A股".to_string(),
        "HK" => "🇭🇰 港股".to_string(),
        _ => m.to_string(),
    };
    let by_market = make_items(
        market_pnl
            .into_iter()
            .map(|(k, v)| (market_label(&k), v))
            .collect(),
    );
    let by_category = make_items(category_pnl);

    let mut by_holding: Vec<AttributionItem> = holding_pnl
        .into_iter()
        .map(|(sym, (_mkt, _cat, pnl, sv, _ev))| {
            let contribution_percent = if total_pnl != 0.0 {
                pnl / total_pnl.abs() * 100.0
            } else {
                0.0
            };
            let weight = if total_start_val != 0.0 {
                sv / total_start_val * 100.0
            } else {
                0.0
            };
            // Use "symbol name" format for display, showing actual stock name
            let display_name = match holding_names.get(&sym) {
                Some(n) if !n.is_empty() && *n != sym => format!("{} {}", sym, n),
                _ => sym,
            };
            AttributionItem {
                name: display_name,
                pnl,
                contribution_percent,
                weight,
            }
        })
        .collect();
    by_holding.sort_by(|a, b| b.pnl.partial_cmp(&a.pnl).unwrap_or(std::cmp::Ordering::Equal));

    Ok(ReturnAttribution {
        total_pnl,
        by_market,
        by_category,
        by_holding,
    })
}

pub fn get_monthly_returns(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<Vec<MonthlyReturn>, String> {
    let daily = fetch_daily_values(db, start_date, end_date, filter)?;
    if daily.is_empty() {
        return Ok(vec![]);
    }

    // Group by year-month, storing (first_date, first_value, first_cost,
    // last_date, last_value, last_cost, last_cum_pnl)
    let mut months: std::collections::BTreeMap<
        (i32, u32),
        (NaiveDate, f64, f64, NaiveDate, f64, f64, f64),
    > = std::collections::BTreeMap::new();

    for (date, value, cost, _, cum_pnl) in &daily {
        let key = (date.year(), date.month());
        months
            .entry(key)
            .and_modify(|e| {
                if *date > e.3 {
                    e.3 = *date;
                    e.4 = *value;
                    e.5 = *cost;
                    e.6 = *cum_pnl;
                }
            })
            .or_insert((*date, *value, *cost, *date, *value, *cost, *cum_pnl));
    }

    // Build a sorted list of month keys
    let keys: Vec<(i32, u32)> = months.keys().cloned().collect();
    let mut result = Vec::new();

    for (i, &key) in keys.iter().enumerate() {
        let (_, first_v, first_cost, _, end_v, end_cost, end_cum_pnl) = months[&key];

        // Monthly return based on PnL delta relative to cost basis,
        // NOT raw value delta (which conflates capital inflows with gains).
        //
        // Return = (cum_pnl_end - cum_pnl_start) / cost_start
        // For the first month, cum_pnl_start = 0.
        let (start_cost, start_cum_pnl) = if i == 0 {
            // Use first day's cost and cum_pnl as baseline
            (first_cost, 0.0_f64) // cum_pnl relative to month start = 0
        } else {
            let prev_key = keys[i - 1];
            let (_, _, _, _, _, prev_cost, prev_cum_pnl) = months[&prev_key];
            (prev_cost, prev_cum_pnl)
        };

        let pnl_delta = end_cum_pnl - start_cum_pnl;
        let return_rate = if start_cost > 0.0 {
            pnl_delta / start_cost * 100.0
        } else {
            0.0
        };

        result.push(MonthlyReturn {
            year: months[&key].3.year(),
            month: months[&key].3.month(),
            return_rate,
            pnl: pnl_delta,
            start_value: first_v,
            end_value: end_v,
        });
    }

    Ok(result)
}

pub fn get_holding_performance_ranking(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    sort_by: &str,
    limit: usize,
    filter: &PerformanceFilter,
) -> Result<Vec<HoldingPerformance>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = start_date.format("%Y-%m-%d").to_string();
    let end_str = end_date.format("%Y-%m-%d").to_string();

    // Collect per-symbol start and end values from snapshots
    struct SnapRow {
        symbol: String,
        market: String,
        category_name: String,
        market_value: f64,
    }

    let fetch_snap = |date_param: &str| -> Result<Vec<SnapRow>, String> {
        let mut sql = String::from(
            "SELECT symbol, market, COALESCE(category_name, '未分类'), SUM(market_value)
             FROM daily_holding_snapshots
             WHERE date = (
                 SELECT MAX(date) FROM daily_holding_snapshots WHERE date <= ?1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(date_param.to_string())];
        filter.append_where_clauses(&mut sql, &mut params);
        sql.push(')');
        filter.append_where_clauses(&mut sql, &mut params);
        sql.push_str(" GROUP BY symbol");
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(SnapRow {
                    symbol: row.get(0)?,
                    market: row.get(1)?,
                    category_name: row.get(2)?,
                    market_value: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    };

    let start_snaps = fetch_snap(&start_str)?;
    let end_snaps = fetch_snap(&end_str)?;

    // Fetch net cash flows per symbol from transactions during the period
    let mut net_cash_flows: std::collections::HashMap<String, f64> =
        std::collections::HashMap::new();
    {
        let actual_start_str = {
            let mut sql = String::from(
                "SELECT MAX(date) FROM daily_holding_snapshots WHERE date <= ?1",
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(start_str.clone())];
            filter.append_where_clauses(&mut sql, &mut params);
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            conn.query_row(&sql, param_refs.as_slice(), |row| row.get::<_, Option<String>>(0))
                .map_err(|e| e.to_string())?
                .unwrap_or_else(|| start_str.clone())
        };
        let mut sql = String::from(
            "SELECT symbol, transaction_type, SUM(total_amount)
             FROM transactions
             WHERE DATE(traded_at) > ?1 AND DATE(traded_at) <= ?2",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(actual_start_str),
            Box::new(end_str.clone()),
        ];
        if let Some(ref account_id) = filter.account_id {
            sql.push_str(&format!(" AND account_id = ?{}", params.len() + 1));
            params.push(Box::new(account_id.clone()));
        }
        if let Some(ref market) = filter.market {
            sql.push_str(&format!(" AND market = ?{}", params.len() + 1));
            params.push(Box::new(market.clone()));
        }
        sql.push_str(" GROUP BY symbol, transaction_type");
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (sym, tx_type, amount) in rows {
            let flow = match tx_type.as_str() {
                "BUY" => amount,
                "OPEN" => amount,
                "SELL" => -amount,
                _ => 0.0,
            };
            *net_cash_flows.entry(sym).or_insert(0.0) += flow;
        }
    }

    let mut start_map: std::collections::HashMap<String, (String, String, f64)> =
        std::collections::HashMap::new();
    for s in start_snaps {
        start_map.insert(s.symbol, (s.market, s.category_name, s.market_value));
    }

    // Fetch avg_cost * shares from holdings for cost basis when sv = 0
    let holding_cost_map: std::collections::HashMap<String, f64> = {
        let mut map = std::collections::HashMap::new();
        let mut stmt = conn
            .prepare("SELECT symbol, avg_cost * shares FROM holdings")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (sym, cost) in rows {
            map.insert(sym, cost);
        }
        map
    };

    let mut performances: Vec<HoldingPerformance> = end_snaps
        .into_iter()
        .filter(|e| !crate::services::quote_service::is_cash_symbol(&e.symbol))
        .map(|e| {
            let (market, cat, sv) = start_map
                .get(&e.symbol)
                .cloned()
                .unwrap_or_else(|| (e.market.clone(), e.category_name.clone(), 0.0));
            let ev = e.market_value;
            let cf = net_cash_flows.get(&e.symbol).copied().unwrap_or(0.0);

            let (actual_sv, cost_base, pnl) = if sv > 0.0 {
                // Had position at start: PnL = end - start - net_cash_flow
                let p = ev - sv - cf;
                (sv, sv + cf.max(0.0), p)
            } else if cf > 0.0 {
                // New position opened during period: cost = cash invested
                (cf, cf, ev - cf)
            } else {
                // Position existed before period, no start snapshot
                let cost = holding_cost_map.get(&e.symbol).copied().unwrap_or(ev);
                (cost, cost, ev - cost)
            };

            let return_rate = if cost_base > 0.0 {
                pnl / cost_base * 100.0
            } else {
                0.0
            };
            HoldingPerformance {
                symbol: e.symbol,
                name: String::new(),
                market,
                category_name: cat,
                return_rate,
                pnl,
                start_value: actual_sv,
                end_value: ev,
            }
        })
        .collect();

    // Enrich with holding names
    {
        let mut name_stmt = conn
            .prepare("SELECT symbol, name FROM holdings")
            .map_err(|e| e.to_string())?;
        let name_map: std::collections::HashMap<String, String> = name_stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect();
        for p in &mut performances {
            if let Some(name) = name_map.get(&p.symbol) {
                p.name = name.clone();
            }
            if p.name.is_empty() {
                p.name = p.symbol.clone();
            }
        }
    }

    // Sort
    if sort_by == "pnl" {
        performances.sort_by(|a, b| {
            b.pnl
                .partial_cmp(&a.pnl)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        performances.sort_by(|a, b| {
            b.return_rate
                .partial_cmp(&a.return_rate)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    Ok(performances.into_iter().take(limit).collect())
}

// ─────────────────────────────────────────────────────────────────────────────
// Benchmark data
// ─────────────────────────────────────────────────────────────────────────────

/// Cache benchmark data in SQLite.
pub fn cache_benchmark_prices(
    db: &Database,
    symbol: &str,
    points: &[BenchmarkDataPoint],
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    for p in points {
        conn.execute(
            "INSERT OR REPLACE INTO benchmark_daily_prices (symbol, date, close_price, change_percent)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![symbol, p.date, p.close_price, p.change_percent],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Read cached benchmark prices from SQLite.
pub fn read_cached_benchmark(
    db: &Database,
    symbol: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<BenchmarkDataPoint>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = start_date.format("%Y-%m-%d").to_string();
    let end_str = end_date.format("%Y-%m-%d").to_string();

    let mut stmt = conn
        .prepare(
            "SELECT date, close_price, change_percent
             FROM benchmark_daily_prices
             WHERE symbol = ?1 AND date BETWEEN ?2 AND ?3
             ORDER BY date ASC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(rusqlite::params![symbol, start_str, end_str], |row| {
            Ok(BenchmarkDataPoint {
                date: row.get(0)?,
                close_price: row.get(1)?,
                change_percent: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(rows)
}

/// Fetch benchmark history from Yahoo Finance and cache it.
pub async fn fetch_benchmark_history(
    db: &Database,
    symbol: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<BenchmarkDataPoint>, String> {
    // Check cache first
    let cached = read_cached_benchmark(db, symbol, start_date, end_date)?;

    // If we have data covering the range, use it
    let days_needed = (end_date - start_date).num_days();
    if (cached.len() as f64) >= days_needed as f64 * CACHE_COVERAGE_THRESHOLD {
        return Ok(cached);
    }

    // Fetch from Yahoo Finance
    let start_ts = start_date
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let end_ts = end_date
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc()
        .timestamp();

    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?period1={}&period2={}&interval=1d",
        symbol, start_ts, end_ts
    );

    let resp = http_client::general_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    let timestamps = json["chart"]["result"][0]["timestamp"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let closes = json["chart"]["result"][0]["indicators"]["quote"][0]["close"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let mut points: Vec<BenchmarkDataPoint> = Vec::new();
    let mut prev_close: Option<f64> = None;

    for (ts, cl) in timestamps.iter().zip(closes.iter()) {
        if let (Some(ts_i), Some(cl_f)) = (ts.as_i64(), cl.as_f64()) {
            let date = chrono::DateTime::from_timestamp(ts_i, 0)
                .unwrap_or_default()
                .date_naive();
            let change_pct = prev_close
                .map(|pc| if pc != 0.0 { (cl_f - pc) / pc * 100.0 } else { 0.0 })
                .unwrap_or(0.0);
            points.push(BenchmarkDataPoint {
                date: date.format("%Y-%m-%d").to_string(),
                close_price: cl_f,
                change_percent: change_pct,
            });
            prev_close = Some(cl_f);
        }
    }

    // Cache the fetched data
    cache_benchmark_prices(db, symbol, &points)?;

    Ok(points)
}

/// Build a return series for the benchmark (cumulative %).
/// When `base_price` is provided, cumulative returns are calculated relative
/// to this price (the closing price on the day before the visible range)
/// instead of the first element in `points`.
pub fn benchmark_to_return_series(
    points: &[BenchmarkDataPoint],
    base_price: Option<f64>,
) -> Vec<ReturnDataPoint> {
    if points.is_empty() {
        return vec![];
    }
    let start_price = base_price.unwrap_or(points[0].close_price);
    let mut prev_price = start_price;
    points
        .iter()
        .map(|p| {
            let daily_return = if prev_price > 0.0 {
                (p.close_price - prev_price) / prev_price * 100.0
            } else {
                0.0
            };
            let cumulative_return = if start_price > 0.0 {
                (p.close_price - start_price) / start_price * 100.0
            } else {
                0.0
            };
            prev_price = p.close_price;
            ReturnDataPoint {
                date: p.date.clone(),
                cumulative_return,
                daily_return,
                portfolio_value: p.close_price,
                daily_pnl: 0.0,
            }
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::performance::{calculate_twr_from_periods, parse_date, SubPeriod};

    #[test]
    fn test_twr_calculation() {
        let periods = vec![
            SubPeriod { start_value: 100.0, end_value: 110.0, cash_flow: 0.0 },
            SubPeriod { start_value: 110.0, end_value: 99.0, cash_flow: 0.0 },
        ];
        let twr = calculate_twr_from_periods(&periods);
        // (110/100 - 1) = 0.1, (99/110 - 1) = -0.1, TWR = 1.1 * 0.9 - 1 = -0.01
        assert!((twr - (-0.01)).abs() < 1e-9);
    }

    #[test]
    fn test_annualise_return() {
        let ar = annualise_return(0.10, 365);
        assert!((ar - 0.10).abs() < 1e-6);

        let ar2 = annualise_return(0.0, 365);
        assert_eq!(ar2, 0.0);
    }

    #[test]
    fn test_volatility() {
        let returns = vec![1.0, -1.0, 2.0, -2.0, 0.5];
        let (dv, av) = calculate_volatility(&returns);
        assert!(dv > 0.0);
        assert!((av - dv * 252.0_f64.sqrt()).abs() < 1e-9);
    }

    #[test]
    fn test_max_drawdown() {
        let series: Vec<ReturnDataPoint> = vec![
            ReturnDataPoint { date: "2024-01-01".to_string(), cumulative_return: 0.0, daily_return: 0.0, portfolio_value: 100.0, daily_pnl: 0.0 },
            ReturnDataPoint { date: "2024-01-02".to_string(), cumulative_return: 10.0, daily_return: 10.0, portfolio_value: 110.0, daily_pnl: 10.0 },
            ReturnDataPoint { date: "2024-01-03".to_string(), cumulative_return: -5.0, daily_return: -15.0, portfolio_value: 95.0, daily_pnl: -15.0 },
            ReturnDataPoint { date: "2024-01-04".to_string(), cumulative_return: 5.0, daily_return: 10.0, portfolio_value: 105.0, daily_pnl: 10.0 },
        ];
        let dd = calculate_max_drawdown(&series);
        // Peak = 110 on day 2, trough = 95 on day 3 → MDD = (95-110)/110 ≈ -13.6%
        assert!(dd.max_drawdown < 0.0);
        assert!((dd.max_drawdown - (-13.636_363_636)).abs() < 0.001);
        assert_eq!(dd.peak_date, "2024-01-02");
        assert_eq!(dd.trough_date, "2024-01-03");
    }

    #[test]
    fn test_build_return_series() {
        // (date, value, cost, daily_pnl, cumulative_pnl)
        let daily = vec![
            (parse_date("2024-01-01").unwrap(), 100.0, 100.0, 0.0, 0.0),
            (parse_date("2024-01-02").unwrap(), 105.0, 100.0, 5.0, 5.0),
            (parse_date("2024-01-03").unwrap(), 103.0, 100.0, -2.0, 3.0),
        ];
        let series = build_return_series(&daily, None);
        assert_eq!(series.len(), 3);
        // Day2: cpnl/cost = 5/100 * 100 = 5%
        assert!((series[1].cumulative_return - 5.0).abs() < 1e-6);
        // Day3: cpnl/cost = 3/100 * 100 = 3%
        assert!((series[2].cumulative_return - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_build_return_series_with_inception_value() {
        // inception_value is now only a legacy parameter; cumulative return
        // is driven by cost-based PnL, not by value delta from base.
        let daily = vec![
            (parse_date("2024-03-01").unwrap(), 100.0, 100.0, 0.0, 0.0),
            (parse_date("2024-03-02").unwrap(), 105.0, 100.0, 5.0, 5.0),
            (parse_date("2024-03-03").unwrap(), 103.0, 100.0, -2.0, 3.0),
        ];
        let series = build_return_series(&daily, Some(50.0));
        assert_eq!(series.len(), 3);
        // cumulative_return = cpnl / cost (inception_value ignored)
        assert!((series[0].cumulative_return - 0.0).abs() < 1e-6);
        assert!((series[1].cumulative_return - 5.0).abs() < 1e-6);
        assert!((series[2].cumulative_return - 3.0).abs() < 1e-6);
        // daily_return should still be day-over-day value change
        assert!((series[0].daily_return - 0.0).abs() < 1e-6);
        // (105 - 100) / 100 * 100 = 5%
        assert!((series[1].daily_return - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_benchmark_to_return_series_no_base() {
        use crate::models::performance::BenchmarkDataPoint;
        let points = vec![
            BenchmarkDataPoint { date: "2024-01-01".into(), close_price: 100.0, change_percent: 0.0 },
            BenchmarkDataPoint { date: "2024-01-02".into(), close_price: 105.0, change_percent: 5.0 },
            BenchmarkDataPoint { date: "2024-01-03".into(), close_price: 103.0, change_percent: -1.9 },
        ];
        let series = benchmark_to_return_series(&points, None);
        assert_eq!(series.len(), 3);
        // Without base_price the first point is the baseline → 0%
        assert!((series[0].cumulative_return - 0.0).abs() < 1e-6);
        assert!((series[1].cumulative_return - 5.0).abs() < 1e-6);
        assert!((series[2].cumulative_return - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_benchmark_to_return_series_with_base() {
        use crate::models::performance::BenchmarkDataPoint;
        // Previous day close was 95 → first visible day already shows a return
        let points = vec![
            BenchmarkDataPoint { date: "2024-01-02".into(), close_price: 100.0, change_percent: 5.26 },
            BenchmarkDataPoint { date: "2024-01-03".into(), close_price: 105.0, change_percent: 5.0 },
        ];
        let series = benchmark_to_return_series(&points, Some(95.0));
        assert_eq!(series.len(), 2);
        // cumulative: (100 - 95) / 95 * 100 ≈ 5.263%
        assert!((series[0].cumulative_return - 5.263_157_894).abs() < 0.001);
        // cumulative: (105 - 95) / 95 * 100 ≈ 10.526%
        assert!((series[1].cumulative_return - 10.526_315_789).abs() < 0.001);
        // daily: (100 - 95) / 95 * 100 ≈ 5.263%
        assert!((series[0].daily_return - 5.263_157_894).abs() < 0.001);
        // daily: (105 - 100) / 100 * 100 = 5%
        assert!((series[1].daily_return - 5.0).abs() < 1e-6);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Additional performance calculation tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_twr_single_period_no_cash_flow() {
        // Simple case: one period, 10% return
        let periods = vec![
            SubPeriod { start_value: 1000.0, end_value: 1100.0, cash_flow: 0.0 },
        ];
        let twr = calculate_twr_from_periods(&periods);
        assert!((twr - 0.10).abs() < 1e-9);
    }

    #[test]
    fn test_twr_with_cash_flow() {
        // Period 1: start 1000, end 1100, no CF → +10%
        // Period 2: start 1600 (1100 + 500 deposit), end 1760, CF=500 → (1760 - 1600 - 500) / 1600 is wrong
        // Actually: period_return = (end - start - CF) / start = (1760 - 1600 - 500) / 1600 = -0.2125
        // Wait, SubPeriod.period_return says (end_value - start_value - cash_flow) / start_value
        // So period 2 start_value should be the value BEFORE the cash flow
        // Let's say: start=1100, deposit 500 brings it to 1600 as end_value, cash_flow=500
        // period_return = (1600 - 1100 - 500) / 1100 = 0.0
        // That makes sense — the growth was entirely from the deposit
        let periods = vec![
            SubPeriod { start_value: 1000.0, end_value: 1100.0, cash_flow: 0.0 },
            SubPeriod { start_value: 1100.0, end_value: 1600.0, cash_flow: 500.0 },
        ];
        let twr = calculate_twr_from_periods(&periods);
        // TWR = (1 + 0.10) * (1 + 0.0) - 1 = 0.10
        assert!((twr - 0.10).abs() < 1e-9);
    }

    #[test]
    fn test_twr_with_loss() {
        let periods = vec![
            SubPeriod { start_value: 1000.0, end_value: 900.0, cash_flow: 0.0 },
            SubPeriod { start_value: 900.0, end_value: 810.0, cash_flow: 0.0 },
        ];
        let twr = calculate_twr_from_periods(&periods);
        // (900/1000) * (810/900) - 1 = 0.9 * 0.9 - 1 = -0.19
        assert!((twr - (-0.19)).abs() < 1e-9);
    }

    #[test]
    fn test_twr_empty_periods() {
        let twr = calculate_twr_from_periods(&[]);
        // product of empty iterator is 1.0, so 1.0 - 1.0 = 0.0
        assert!((twr - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_twr_zero_start_value_returns_zero() {
        let periods = vec![
            SubPeriod { start_value: 0.0, end_value: 100.0, cash_flow: 0.0 },
        ];
        let twr = calculate_twr_from_periods(&periods);
        // period_return returns 0.0 when start_value is 0, so TWR = (1+0) - 1 = 0
        assert!((twr - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_annualise_return_half_year() {
        // 5% return in 182.5 days → annualised ≈ (1.05)^2 - 1 ≈ 10.25%
        let ar = annualise_return(0.05, 183);
        // (1.05)^(365/183) - 1 ≈ 0.10189
        assert!(ar > 0.10 && ar < 0.11);
    }

    #[test]
    fn test_annualise_return_zero_days() {
        assert_eq!(annualise_return(0.10, 0), 0.0);
        assert_eq!(annualise_return(0.10, -5), 0.0);
    }

    #[test]
    fn test_volatility_constant_returns() {
        // All same returns → variance = 0
        let returns = vec![1.0, 1.0, 1.0, 1.0, 1.0];
        let (dv, av) = calculate_volatility(&returns);
        assert!((dv - 0.0).abs() < 1e-9);
        assert!((av - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_volatility_single_return() {
        let (dv, av) = calculate_volatility(&[5.0]);
        assert_eq!(dv, 0.0);
        assert_eq!(av, 0.0);
    }

    #[test]
    fn test_volatility_empty() {
        let (dv, av) = calculate_volatility(&[]);
        assert_eq!(dv, 0.0);
        assert_eq!(av, 0.0);
    }

    #[test]
    fn test_sharpe_ratio_basic() {
        // 15% annual return, 4.5% risk-free, 10% vol → Sharpe = (0.15 - 0.045) / 0.10 = 1.05
        let sharpe = calculate_sharpe(0.15, 0.045, 0.10);
        assert!((sharpe - 1.05).abs() < 1e-9);
    }

    #[test]
    fn test_sharpe_ratio_zero_vol() {
        let sharpe = calculate_sharpe(0.15, 0.045, 0.0);
        assert_eq!(sharpe, 0.0);
    }

    #[test]
    fn test_sharpe_ratio_negative_excess_return() {
        // Return below risk-free → negative Sharpe
        let sharpe = calculate_sharpe(0.02, 0.045, 0.10);
        assert!(sharpe < 0.0);
        assert!((sharpe - (-0.25)).abs() < 1e-9);
    }

    #[test]
    fn test_max_drawdown_no_drawdown() {
        // Monotonically increasing portfolio
        let series: Vec<ReturnDataPoint> = vec![
            ReturnDataPoint { date: "2024-01-01".into(), cumulative_return: 0.0, daily_return: 0.0, portfolio_value: 100.0, daily_pnl: 0.0 },
            ReturnDataPoint { date: "2024-01-02".into(), cumulative_return: 5.0, daily_return: 5.0, portfolio_value: 105.0, daily_pnl: 5.0 },
            ReturnDataPoint { date: "2024-01-03".into(), cumulative_return: 10.0, daily_return: 5.0, portfolio_value: 110.0, daily_pnl: 5.0 },
        ];
        let dd = calculate_max_drawdown(&series);
        assert!((dd.max_drawdown - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_max_drawdown_empty() {
        let dd = calculate_max_drawdown(&[]);
        assert!((dd.max_drawdown - 0.0).abs() < 1e-9);
        assert_eq!(dd.drawdown_duration, 0);
    }

    #[test]
    fn test_max_drawdown_with_recovery() {
        let series: Vec<ReturnDataPoint> = vec![
            ReturnDataPoint { date: "2024-01-01".into(), cumulative_return: 0.0, daily_return: 0.0, portfolio_value: 100.0, daily_pnl: 0.0 },
            ReturnDataPoint { date: "2024-01-02".into(), cumulative_return: 20.0, daily_return: 20.0, portfolio_value: 120.0, daily_pnl: 20.0 },
            ReturnDataPoint { date: "2024-01-03".into(), cumulative_return: 0.0, daily_return: -16.67, portfolio_value: 100.0, daily_pnl: -20.0 },
            ReturnDataPoint { date: "2024-01-04".into(), cumulative_return: 25.0, daily_return: 25.0, portfolio_value: 125.0, daily_pnl: 25.0 },
        ];
        let dd = calculate_max_drawdown(&series);
        // Peak=120, trough=100 → (100-120)/120 = -16.67%
        assert!((dd.max_drawdown - (-16.666_666_667)).abs() < 0.01);
        assert_eq!(dd.peak_date, "2024-01-02");
        assert_eq!(dd.trough_date, "2024-01-03");
        // Recovery at day 4 when value=125 >= peak of 120
        assert_eq!(dd.recovery_date, Some("2024-01-04".to_string()));
    }

    #[test]
    fn test_build_return_series_empty() {
        let series = build_return_series(&[], None);
        assert!(series.is_empty());
    }

    #[test]
    fn test_build_return_series_single_point() {
        let daily = vec![
            (parse_date("2024-01-01").unwrap(), 100.0, 100.0, 0.0, 0.0),
        ];
        let series = build_return_series(&daily, None);
        assert_eq!(series.len(), 1);
        assert!((series[0].cumulative_return - 0.0).abs() < 1e-6);
        assert!((series[0].daily_return - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_build_return_series_zero_start_value() {
        // When cost is 0 (e.g., before any investment), return should be 0
        let daily = vec![
            (parse_date("2024-01-01").unwrap(), 0.0, 0.0, 0.0, 0.0),
            (parse_date("2024-01-02").unwrap(), 100.0, 100.0, 100.0, 0.0),
        ];
        let series = build_return_series(&daily, None);
        // cost=0 → cumulative_return guarded to 0
        assert!((series[0].cumulative_return - 0.0).abs() < 1e-6);
        // Day2: cpnl=0/cost=100 = 0%
        assert!((series[1].cumulative_return - 0.0).abs() < 1e-6);
    }
}
