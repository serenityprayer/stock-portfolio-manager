//! P4 — what-if stress test. Pure preview: never writes HWM or risk events.

use crate::db::Database;
use crate::models::enhancement::*;
use crate::models::risk::{SEV_BLOCK, SEV_CRITICAL, SEV_WARNING};
use crate::services::{risk_engine, risk_service};
use std::collections::HashMap;

/// `symbol = "*"` applies a market-wide shock; a specific symbol override
/// takes precedence over the global one regardless of order.
pub fn resolve_overrides(
    overrides: &[StressOverride],
) -> Result<(Option<f64>, HashMap<String, f64>), String> {
    let mut by_symbol: HashMap<String, f64> = HashMap::new();
    let mut global: Option<f64> = None;
    for o in overrides {
        if !(o.price_change_percent.is_finite() && o.price_change_percent >= -100.0) {
            return Err("价格冲击必须是 ≥ -100% 的有限数字".to_string());
        }
        if o.symbol.trim() == "*" {
            global = Some(o.price_change_percent);
        } else {
            by_symbol.insert(o.symbol.trim().to_uppercase(), o.price_change_percent);
        }
    }
    Ok((global, by_symbol))
}

pub fn run(db: &Database, overrides: &[StressOverride]) -> Result<StressResult, String> {
    let mut ctx = risk_service::build_context_with_options(db, false)?;
    let hwm = risk_service::get_hwm(db)?;
    let (global, by_symbol) = resolve_overrides(overrides)?;

    let baseline_total_assets = ctx.total_assets_base;
    let peak = ctx.peak_unit_nav;
    let baseline_drawdown = if peak > 0.0 {
        (ctx.unit_nav - peak) / peak * 100.0
    } else {
        0.0
    };

    let mut deltas = Vec::new();
    for p in ctx.positions.iter_mut() {
        if p.is_cash {
            continue;
        }
        let pct = by_symbol
            .get(&p.symbol.to_uppercase())
            .copied()
            .or(global);
        let Some(pct) = pct else { continue };
        let factor = (1.0 + pct / 100.0).max(0.0);
        let old_price = p.price;
        let old_value = p.market_value_base;
        let new_price = old_price * factor;
        let new_value = old_value * factor;

        p.price = new_price;
        p.market_value_base = new_value;
        p.pnl_percent = if p.avg_cost > 0.0 {
            (new_price - p.avg_cost) / p.avg_cost * 100.0
        } else {
            0.0
        };
        ctx.total_assets_base += new_value - old_value;
        deltas.push(StressDelta {
            symbol: p.symbol.clone(),
            old_price,
            new_price,
            old_value,
            new_value,
        });
    }

    // Same unit-NAV formula as `update_nav_state`; HWM (peak) is a historical
    // fact and is deliberately NOT moved by a hypothetical scenario.
    let stressed_nav = if hwm.base_value > 0.0 {
        let u = (ctx.total_assets_base - hwm.net_external_flow) / hwm.base_value;
        if u.is_finite() && u > 0.0 { u } else { ctx.unit_nav }
    } else {
        ctx.unit_nav
    };
    ctx.unit_nav = stressed_nav;
    let stressed_drawdown = if peak > 0.0 {
        (stressed_nav - peak) / peak * 100.0
    } else {
        0.0
    };

    let findings = risk_engine::evaluate_all(&ctx);
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

    Ok(StressResult {
        as_of: ctx.as_of,
        baseline_total_assets,
        stressed_total_assets: ctx.total_assets_base,
        baseline_drawdown,
        stressed_drawdown,
        status: status.to_string(),
        findings,
        deltas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specific_symbol_beats_global_shock() {
        let (global, by_symbol) = resolve_overrides(&[
            StressOverride { symbol: "*".into(), price_change_percent: -10.0 },
            StressOverride { symbol: "pdd".into(), price_change_percent: -30.0 },
        ])
        .unwrap();
        assert_eq!(global, Some(-10.0));
        assert_eq!(by_symbol.get("PDD").copied().or(global), Some(-30.0));
        assert_eq!(by_symbol.get("RDDT").copied().or(global), Some(-10.0));
    }

    #[test]
    fn invalid_shock_is_rejected() {
        assert!(resolve_overrides(&[StressOverride {
            symbol: "*".into(),
            price_change_percent: -101.0
        }])
        .is_err());
        assert!(resolve_overrides(&[StressOverride {
            symbol: "*".into(),
            price_change_percent: f64::NAN
        }])
        .is_err());
        assert!(resolve_overrides(&[]).unwrap().0.is_none());
    }
}
