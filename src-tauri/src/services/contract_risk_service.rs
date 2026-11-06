//! P4 — contract liquidation-distance rule (design doc §14 item 3).
//!
//! Contracts are deliberately kept OUT of the three spot iron laws: a 5x short
//! and a 1x long have completely different risk semantics. This module only
//! computes a distance-to-liquidation preview; it never writes risk events.
//!
//! Price model is an isolated-margin approximation with a 0.5% maintenance
//! rate. It is a monitoring signal, not an exchange-accurate liquidation price.

use crate::models::enhancement::{ContractPriceInput, ContractRiskItem};
use crate::models::risk::{SEV_BLOCK, SEV_CRITICAL, SEV_INFO};
use rusqlite::{params, Connection};
use std::collections::HashMap;

const MAINTENANCE_RATE: f64 = 0.005;
const RULE_ID: &str = "CONTRACT_LIQUIDATION_DISTANCE";
const DEFAULT_THRESHOLD: f64 = 20.0;

/// Long: liquidation below open. Short: liquidation above open.
/// For 1x longs there is effectively no liquidation (distance ≈ 100%).
pub fn liquidation_price(open: f64, leverage: f64, side: &str) -> f64 {
    let lev = leverage.max(1.0);
    let move_pct = (1.0 / lev - MAINTENANCE_RATE).max(0.0);
    match side {
        "short" => open * (1.0 + move_pct),
        _ => open * (1.0 - move_pct),
    }
}

pub fn distance_percent(current: f64, liq: f64, side: &str) -> f64 {
    if current <= 0.0 {
        return 0.0;
    }
    if side == "short" {
        (liq - current) / current * 100.0
    } else {
        (current - liq) / current * 100.0
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_item(
    id: &str,
    symbol: &str,
    side: &str,
    leverage: f64,
    open: f64,
    current: f64,
    has_quote: bool,
    threshold: f64,
    rule_enabled: bool,
) -> ContractRiskItem {
    let liq = liquidation_price(open, leverage, side);
    let distance = distance_percent(current, liq, side);
    let (severity, message) = if !has_quote {
        (
            SEV_INFO.to_string(),
            "缺少实时报价，距离按开仓价估算，仅供参考".to_string(),
        )
    } else if !rule_enabled {
        (
            SEV_INFO.to_string(),
            format!("强平规则未启用（阈值 {threshold:.0}%）"),
        )
    } else if distance <= 0.0 {
        (SEV_BLOCK.to_string(), "已跌破/涨破预估强平价".to_string())
    } else if distance < threshold {
        (
            SEV_CRITICAL.to_string(),
            format!("距强平仅 {distance:.1}%，低于 {threshold:.0}% 警戒线"),
        )
    } else if leverage <= 1.0 {
        (SEV_INFO.to_string(), "1x 类现货仓位，无强平风险".to_string())
    } else {
        (SEV_INFO.to_string(), format!("距强平 {distance:.1}%"))
    };
    ContractRiskItem {
        id: id.to_string(),
        symbol: symbol.to_string(),
        position_type: side.to_string(),
        leverage,
        open_price: open,
        current_price: current,
        liquidation_price: liq,
        distance_percent: distance,
        severity,
        message,
        has_quote,
    }
}

pub fn evaluate(
    conn: &Connection,
    prices: &[ContractPriceInput],
) -> Result<Vec<ContractRiskItem>, String> {
    let (enabled, threshold) = conn
        .query_row(
            "SELECT enabled, COALESCE(threshold, 20.0) FROM risk_rules WHERE id = ?1",
            params![RULE_ID],
            |r| Ok((r.get::<_, i32>(0)? != 0, r.get::<_, f64>(1)?)),
        )
        .unwrap_or((false, DEFAULT_THRESHOLD));

    let mut provided: HashMap<String, f64> = HashMap::new();
    for p in prices {
        if p.price > 0.0 && p.price.is_finite() {
            provided.insert(p.symbol.trim().to_uppercase(), p.price);
        }
    }
    let mut cached: HashMap<String, (f64, String)> = HashMap::new();
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
            if price <= 0.0 {
                continue;
            }
            let key = sym.to_uppercase();
            let fresh = cached.get(&key).map(|(_, old)| ts > *old).unwrap_or(true);
            if fresh {
                cached.insert(key, (price, ts));
            }
        }
    }

    let rows: Vec<(String, String, String, f64, f64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, symbol, position_type, open_price, leverage
                 FROM crypto_contract WHERE shares > 0 ORDER BY symbol",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };

    let mut out = Vec::new();
    for (id, symbol, side, open, leverage) in rows {
        let upper = symbol.to_uppercase();
        let (current, has_quote) = match provided
            .get(&upper)
            .copied()
            .or_else(|| cached.get(&upper).map(|(p, _)| *p))
        {
            Some(p) => (p, true),
            None => (open, false),
        };
        out.push(build_item(
            &id, &symbol, &side, leverage, open, current, has_quote, threshold, enabled,
        ));
    }
    out.sort_by(|a, b| {
        a.distance_percent
            .partial_cmp(&b.distance_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_liquidation_is_below_open_short_is_above() {
        let long = liquidation_price(100.0, 5.0, "long");
        let short = liquidation_price(100.0, 5.0, "short");
        assert!(long < 100.0 && long > 79.0);
        assert!(short > 100.0 && short < 121.0);
    }

    #[test]
    fn one_x_long_has_no_liquidation_risk() {
        let liq = liquidation_price(100.0, 1.0, "long");
        assert!(liq < 1.0);
        assert!(distance_percent(100.0, liq, "long") > 99.0);
    }

    #[test]
    fn distance_shrinks_as_price_moves_against_position() {
        let liq = liquidation_price(100.0, 5.0, "long");
        let near = distance_percent(85.0, liq, "long");
        let far = distance_percent(120.0, liq, "long");
        assert!(near < far);
    }

    #[test]
    fn short_distance_uses_inverse_sign() {
        let liq = liquidation_price(100.0, 5.0, "short");
        assert!(distance_percent(110.0, liq, "short") < distance_percent(90.0, liq, "short"));
    }

    #[test]
    fn critical_and_block_severity() {
        let item = build_item("c1", "ETH", "short", 5.0, 100.0, 118.0, true, 20.0, true);
        assert_eq!(item.severity, SEV_CRITICAL);
        let crossed = build_item("c2", "ETH", "short", 5.0, 100.0, 130.0, true, 20.0, true);
        assert_eq!(crossed.severity, SEV_BLOCK);
    }

    #[test]
    fn disabled_rule_never_escalates() {
        let item = build_item("c1", "ETH", "short", 5.0, 100.0, 118.0, true, 20.0, false);
        assert_eq!(item.severity, SEV_INFO);
    }

    #[test]
    fn missing_quote_falls_back_to_open_price() {
        let item = build_item("c1", "BNB", "long", 5.0, 100.0, 100.0, false, 20.0, true);
        assert_eq!(item.severity, SEV_INFO);
        assert!(!item.has_quote);
    }
}
