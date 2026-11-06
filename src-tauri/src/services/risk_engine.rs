//! Pure rule engine — no IO, no global state, fully reproducible.
//!
//! `evaluate_all(&RiskContext) -> Vec<RiskFinding>` is deliberately a pure
//! function so the same context can be replayed in tests, stress tests and
//! future "what-if" previews without touching the database.

use crate::models::risk::{
    RiskContext, RiskFinding, RiskRuleConfig, SEV_BLOCK, SEV_CRITICAL, SEV_INFO, SEV_WARNING,
};
use serde_json::json;

pub const RULE_STOP_LOSS: &str = "STOP_LOSS";
pub const RULE_MAX_DRAWDOWN: &str = "MAX_DRAWDOWN";
pub const RULE_CONCENTRATION: &str = "CONCENTRATION";
pub const RULE_STALE_DATA: &str = "STALE_DATA";

pub fn severity_rank(s: &str) -> i32 {
    match s {
        SEV_BLOCK => 4,
        SEV_CRITICAL => 3,
        SEV_WARNING => 2,
        SEV_INFO => 1,
        _ => 0,
    }
}

fn enabled<'a>(
    rules: &'a std::collections::HashMap<String, RiskRuleConfig>,
    id: &str,
) -> Option<&'a RiskRuleConfig> {
    rules.get(id).filter(|r| r.enabled)
}

/// Whether a position's quote is trustworthy enough to drive a trading rule.
/// A missing/zero quote is NEVER treated as price 0 (design doc §5.5).
fn quote_usable(pos: &crate::models::risk::RiskPosition, ctx: &RiskContext) -> bool {
    pos.has_quote
        && pos.price > 0.0
        && pos.quote_age_secs <= ctx.freshness.stale_threshold_secs
}

pub fn evaluate_all(ctx: &RiskContext) -> Vec<RiskFinding> {
    let mut findings = Vec::new();

    // ── 铁律 1：止损红线 ────────────────────────────────────────────
    if let Some(rule) = enabled(&ctx.rules, RULE_STOP_LOSS) {
        let limit = rule.threshold.unwrap_or(-20.0);
        for p in ctx.positions.iter().filter(|p| !p.is_cash && p.shares > 0.0) {
            if !quote_usable(p, ctx) {
                // 行情缺失/过期 → 跳过判定，由 STALE_DATA 统一降级提示，
                // 绝不能用脏价格触发斩仓建议。
                continue;
            }
            if p.pnl_percent <= limit {
                findings.push(RiskFinding {
                    rule_id: RULE_STOP_LOSS.to_string(),
                    severity: SEV_CRITICAL.to_string(),
                    symbol: Some(p.symbol.clone()),
                    metric: p.pnl_percent,
                    threshold: limit,
                    message: format!(
                        "{} 浮亏 {:.2}%，已触及止损线 {:.0}%",
                        p.symbol, p.pnl_percent, limit
                    ),
                    context: json!({
                        "name": p.name,
                        "market": p.market,
                        "account_id": p.account_id,
                        "currency": p.currency,
                        "price": p.price,
                        "avg_cost": p.avg_cost,
                        "shares": p.shares,
                        "quote_age_secs": p.quote_age_secs,
                    }),
                });
            }
        }
    }

    // ── 铁律 2：单位净值回撤熔断 ────────────────────────────────────
    // unit_nav 已剔除外部现金流，因此入金不会掩盖回撤、出金不会误触熔断。
    if let Some(rule) = enabled(&ctx.rules, RULE_MAX_DRAWDOWN) {
        let limit = rule.threshold.unwrap_or(-5.0);
        if ctx.peak_unit_nav > 0.0 && ctx.unit_nav > 0.0 && ctx.total_assets_base > 0.0 {
            let dd = (ctx.unit_nav - ctx.peak_unit_nav) / ctx.peak_unit_nav * 100.0;
            if dd <= limit {
                findings.push(RiskFinding {
                    rule_id: RULE_MAX_DRAWDOWN.to_string(),
                    severity: SEV_BLOCK.to_string(),
                    symbol: None,
                    metric: dd,
                    threshold: limit,
                    message: format!(
                        "组合回撤 {:.2}%，已触及 {:.0}% 熔断线，暂停新开仓",
                        dd, limit
                    ),
                    context: json!({
                        "unit_nav": ctx.unit_nav,
                        "peak_unit_nav": ctx.peak_unit_nav,
                        "total_assets_base": ctx.total_assets_base,
                    }),
                });
            }
        }
    }

    // ── 铁律 3：个股集中度 ──────────────────────────────────────────
    // 分母含现金（total_assets_base），分子排除现金。
    if let Some(rule) = enabled(&ctx.rules, RULE_CONCENTRATION) {
        let limit = rule.threshold.unwrap_or(15.0);
        if ctx.total_assets_base > 0.0 {
            for p in ctx.positions.iter().filter(|p| !p.is_cash && p.shares > 0.0) {
                if !quote_usable(p, ctx) {
                    continue;
                }
                let weight = p.market_value_base / ctx.total_assets_base * 100.0;
                if weight > limit {
                    findings.push(RiskFinding {
                        rule_id: RULE_CONCENTRATION.to_string(),
                        severity: SEV_WARNING.to_string(),
                        symbol: Some(p.symbol.clone()),
                        metric: weight,
                        threshold: limit,
                        message: format!(
                            "{} 占总资产 {:.2}%，超过 {:.0}% 上限",
                            p.symbol, weight, limit
                        ),
                        context: json!({
                            "name": p.name,
                            "market": p.market,
                            "account_id": p.account_id,
                            "currency": p.currency,
                            "market_value_base": p.market_value_base,
                            "total_assets_base": ctx.total_assets_base,
                        }),
                    });
                }
            }
        }
    }

    // ── 数据质量降级提示（不阻断） ──────────────────────────────────
    if let Some(rule) = enabled(&ctx.rules, RULE_STALE_DATA) {
        let limit_min = rule.threshold.unwrap_or(1440.0);
        let bad_count = ctx.freshness.missing_symbols.len() + ctx.freshness.stale_symbols.len();
        if bad_count > 0 {
            let mut symbols = ctx.freshness.missing_symbols.clone();
            symbols.extend(ctx.freshness.stale_symbols.clone());
            findings.push(RiskFinding {
                rule_id: RULE_STALE_DATA.to_string(),
                severity: SEV_INFO.to_string(),
                symbol: None,
                metric: ctx.freshness.oldest_quote_age_secs as f64 / 60.0,
                threshold: limit_min,
                message: format!(
                    "{} 个标的行情缺失或超过 {:.0} 分钟未更新，相关规则已跳过判定",
                    bad_count, limit_min
                ),
                context: json!({
                    "symbols": symbols,
                    "missing": ctx.freshness.missing_symbols,
                    "stale": ctx.freshness.stale_symbols,
                }),
            });
        }
    }

    findings.sort_by(|a, b| {
        severity_rank(&b.severity)
            .cmp(&severity_rank(&a.severity))
            .then(a.rule_id.cmp(&b.rule_id))
            .then(a.symbol.cmp(&b.symbol))
    });
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::risk::{QuoteFreshness, RiskPosition, RiskRuleConfig};
    use std::collections::HashMap;

    fn rule(id: &str, threshold: Option<f64>, severity: &str) -> RiskRuleConfig {
        RiskRuleConfig {
            id: id.to_string(),
            name: id.to_string(),
            description: None,
            severity: severity.to_string(),
            enabled: true,
            threshold,
            params: None,
            updated_at: String::new(),
        }
    }

    fn position(symbol: &str, pnl: f64, mv: f64, has_quote: bool, age: i64) -> RiskPosition {
        RiskPosition {
            symbol: symbol.to_string(),
            name: symbol.to_string(),
            market: "US".to_string(),
            account_id: "acc".to_string(),
            currency: "USD".to_string(),
            shares: 10.0,
            avg_cost: 100.0,
            price: if has_quote { 100.0 + pnl } else { 0.0 },
            market_value_base: mv,
            pnl_percent: pnl,
            has_quote,
            quote_age_secs: age,
            is_cash: false,
        }
    }

    fn base_ctx() -> RiskContext {
        let mut rules = HashMap::new();
        rules.insert(RULE_STOP_LOSS.into(), rule(RULE_STOP_LOSS, Some(-20.0), SEV_CRITICAL));
        rules.insert(RULE_MAX_DRAWDOWN.into(), rule(RULE_MAX_DRAWDOWN, Some(-5.0), SEV_BLOCK));
        rules.insert(RULE_CONCENTRATION.into(), rule(RULE_CONCENTRATION, Some(15.0), SEV_WARNING));
        rules.insert(RULE_STALE_DATA.into(), rule(RULE_STALE_DATA, Some(60.0), SEV_INFO));
        RiskContext {
            as_of: "2026-11-07T00:00:00Z".to_string(),
            base_currency: "USD".to_string(),
            positions: vec![],
            cash_value_base: 0.0,
            total_assets_base: 1000.0,
            unit_nav: 1.0,
            peak_unit_nav: 1.0,
            rules,
            freshness: QuoteFreshness {
                has_quotes: true,
                stale_symbols: vec![],
                missing_symbols: vec![],
                oldest_quote_age_secs: 0,
                stale_threshold_secs: 3600,
            },
        }
    }

    #[test]
    fn stop_loss_triggers_only_below_threshold() {
        let mut ctx = base_ctx();
        ctx.positions = vec![
            position("OK", -19.9, 100.0, true, 0),
            position("BAD", -25.0, 100.0, true, 0),
        ];
        let f = evaluate_all(&ctx);
        let hits: Vec<_> = f.iter().filter(|x| x.rule_id == RULE_STOP_LOSS).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].symbol.as_deref(), Some("BAD"));
    }

    #[test]
    fn stale_quote_never_triggers_stop_loss() {
        let mut ctx = base_ctx();
        // 无行情时 price=0，若错误参与判定会被当成 -100% 亏损
        ctx.positions = vec![position("NOQUOTE", 0.0, 100.0, false, 999_999)];
        ctx.freshness.missing_symbols = vec!["NOQUOTE".into()];
        let f = evaluate_all(&ctx);
        assert!(!f.iter().any(|x| x.rule_id == RULE_STOP_LOSS));
        assert!(f.iter().any(|x| x.rule_id == RULE_STALE_DATA));
    }

    #[test]
    fn drawdown_uses_unit_nav_not_total_assets() {
        let mut ctx = base_ctx();
        // 总资产因入金翻倍，但单位净值不变 → 不应触发熔断
        ctx.total_assets_base = 2000.0;
        ctx.unit_nav = 1.0;
        ctx.peak_unit_nav = 1.0;
        assert!(!evaluate_all(&ctx).iter().any(|x| x.rule_id == RULE_MAX_DRAWDOWN));

        ctx.unit_nav = 0.94;
        let f = evaluate_all(&ctx);
        assert!(f.iter().any(|x| x.rule_id == RULE_MAX_DRAWDOWN && x.severity == SEV_BLOCK));
    }

    #[test]
    fn concentration_uses_cash_inclusive_denominator() {
        let mut ctx = base_ctx();
        ctx.total_assets_base = 1000.0;
        ctx.positions = vec![position("BIG", 1.0, 180.0, true, 0)];
        assert!(evaluate_all(&ctx).iter().any(|x| x.rule_id == RULE_CONCENTRATION));
    }
}
