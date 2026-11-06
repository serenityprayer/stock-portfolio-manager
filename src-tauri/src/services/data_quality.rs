//! P4 — data-quality guards (design doc §11.2).
//!
//! Pure validators are reused at write time; `check` is a read-only health
//! scan the UI can run on demand. Dirty data must never be able to trigger a
//! false circuit breaker.

use crate::models::enhancement::{DataQualityReport, QualityIssue};
use rusqlite::Connection;
use std::collections::HashMap;

pub const MIN_PRICE: f64 = 0.01;
pub const MAX_PRICE: f64 = 100_000.0;
pub const MAX_PRICE_MOVE: f64 = 0.95;
pub const MAX_RATE_MOVE: f64 = 0.20;
pub const MAX_NAV_MOVE: f64 = 0.95;

/// Validate a quote before writing it to `cached_quotes`.
/// `old` is the previously cached price when available.
pub fn validate_price(old: Option<f64>, new: f64) -> Result<(), String> {
    if !new.is_finite() || new < MIN_PRICE || new > MAX_PRICE {
        return Err(format!("价格 {new} 超出合理范围 [{MIN_PRICE}, {MAX_PRICE}]"));
    }
    if let Some(prev) = old.filter(|p| *p > 0.0 && p.is_finite()) {
        let move_pct = (new - prev).abs() / prev;
        if move_pct > MAX_PRICE_MOVE {
            return Err(format!(
                "单标的价格跳变 {:.1}%（{} → {}），拒绝写入",
                move_pct * 100.0,
                prev,
                new
            ));
        }
    }
    Ok(())
}

/// USD/CNY and HKD/CNY sanity ranges from the reference material.
pub fn validate_rate(kind: &str, value: f64, prev: Option<f64>) -> Result<(), String> {
    if !value.is_finite() || value <= 0.0 {
        return Err("汇率必须为正数".to_string());
    }
    let (lo, hi) = match kind {
        "usd_cny" => (5.0, 10.0),
        "cny_hkd" | "hkd_cny" => (0.7, 1.2),
        _ => (0.0001, 100_000.0),
    };
    if value < lo || value > hi {
        return Err(format!("{kind} = {value} 超出合理范围 [{lo}, {hi}]"));
    }
    if let Some(p) = prev.filter(|p| *p > 0.0) {
        if (value - p).abs() / p > MAX_RATE_MOVE {
            return Err(format!("{kind} 日变动超过 20%，拒绝写入"));
        }
    }
    Ok(())
}

/// Prevent a bad quote from permanently locking the high-water mark.
pub fn validate_unit_nav(prev: f64, next: f64) -> Result<(), String> {
    if prev <= 0.0 || !next.is_finite() {
        return Ok(());
    }
    if (next / prev - 1.0).abs() > MAX_NAV_MOVE {
        return Err(format!(
            "单位净值跳变 {:.1}%（{prev:.4} → {next:.4}），拒绝保存快照",
            (next / prev - 1.0) * 100.0
        ));
    }
    Ok(())
}

/// Read-only scan of cached quotes, exchange rates and NAV series.
pub fn check(conn: &Connection) -> Result<DataQualityReport, String> {
    let mut issues: Vec<QualityIssue> = Vec::new();
    let mut checked = 0i64;

    let quotes: Vec<(String, f64, f64, String)> = {
        let mut stmt = conn
            .prepare("SELECT symbol, current_price, previous_close, updated_at FROM cached_quotes")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };

    let mut by_lower: HashMap<String, Vec<String>> = HashMap::new();
    for (symbol, price, prev_close, updated_at) in &quotes {
        checked += 1;
        if let Err(e) = validate_price(None, *price) {
            issues.push(QualityIssue {
                kind: "price_range".into(),
                symbol: symbol.clone(),
                message: e,
            });
        }
        if let Err(e) = validate_price(Some(*prev_close).filter(|p| *p > 0.0), *price) {
            issues.push(QualityIssue {
                kind: "price_jump".into(),
                symbol: symbol.clone(),
                message: e,
            });
        }
        if crate::services::risk_service::quote_age_secs(updated_at) > 24 * 3600 {
            issues.push(QualityIssue {
                kind: "quote_stale".into(),
                symbol: symbol.clone(),
                message: format!("行情超过 24 小时未更新（{updated_at}）"),
            });
        }
        by_lower
            .entry(symbol.to_lowercase())
            .or_default()
            .push(symbol.clone());
    }
    for (lower, names) in by_lower {
        if names.len() > 1 {
            issues.push(QualityIssue {
                kind: "duplicate_symbol".into(),
                symbol: lower.to_uppercase(),
                message: format!("存在大小写重复行情行：{}", names.join(" / ")),
            });
        }
    }

    checked += 1;
    let rate: Option<(f64, f64, f64)> = conn
        .query_row(
            "SELECT usd_cny, usd_hkd, cny_hkd FROM cached_exchange_rates WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    if let Some((usd_cny, usd_hkd, cny_hkd)) = rate {
        for (kind, v) in [
            ("usd_cny", usd_cny),
            ("usd_hkd", usd_hkd),
            ("cny_hkd", cny_hkd),
        ] {
            if let Err(e) = validate_rate(kind, v, None) {
                issues.push(QualityIssue {
                    kind: "rate_range".into(),
                    symbol: kind.into(),
                    message: e,
                });
            }
        }
    }

    checked += 1;
    let navs: Vec<(String, f64)> = {
        let mut stmt = conn
            .prepare("SELECT date, total_value FROM daily_portfolio_values ORDER BY date DESC LIMIT 2")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    if navs.len() == 2 && navs[1].1 > 0.0 && (navs[0].1 / navs[1].1 - 1.0).abs() > MAX_NAV_MOVE {
        issues.push(QualityIssue {
            kind: "nav_jump".into(),
            symbol: navs[0].0.clone(),
            message: format!("日净值跳变 {:.1}%，可能混入脏行情", (navs[0].1 / navs[1].1 - 1.0) * 100.0),
        });
    }

    let rejected = issues.len() as i64;
    Ok(DataQualityReport {
        checked,
        rejected,
        issues,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_positive_and_absurd_prices() {
        assert!(validate_price(None, 0.0).is_err());
        assert!(validate_price(None, -1.0).is_err());
        assert!(validate_price(None, 1_000_000.0).is_err());
        assert!(validate_price(None, 100.0).is_ok());
    }

    #[test]
    fn rejects_95_percent_jump_but_allows_normal_moves() {
        assert!(validate_price(Some(100.0), 196.0).is_err());
        assert!(validate_price(Some(100.0), 1.0).is_err());
        assert!(validate_price(Some(100.0), 130.0).is_ok());
    }

    #[test]
    fn no_previous_price_means_first_observation() {
        assert!(validate_price(None, 0.5).is_ok());
        assert!(validate_price(Some(0.0), 0.5).is_ok());
    }

    #[test]
    fn rate_ranges_follow_the_reference_material() {
        assert!(validate_rate("usd_cny", 7.2, None).is_ok());
        assert!(validate_rate("usd_cny", 1.0, None).is_err());
        assert!(validate_rate("cny_hkd", 1.08, None).is_ok());
        assert!(validate_rate("cny_hkd", 5.0, None).is_err());
        assert!(validate_rate("usd_cny", 8.8, Some(7.2)).is_err());
    }

    #[test]
    fn nav_jump_is_rejected() {
        assert!(validate_unit_nav(1.0, 2.1).is_err());
        assert!(validate_unit_nav(1.0, 0.94).is_ok());
        assert!(validate_unit_nav(0.0, 5.0).is_ok(), "无基线时不做判断");
    }
}
