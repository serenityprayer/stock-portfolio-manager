use crate::models::valuation::{HoldingValuation, PortfolioValuation};
use crate::models::StockQuote;

/// Compute portfolio-level weighted PE and dividend yield from holdings and
/// their current quotes.
pub fn compute_valuation(
    holdings: &[crate::models::HoldingDetail],
    quotes: &std::collections::HashMap<String, StockQuote>,
) -> PortfolioValuation {
    let total_mv: f64 = holdings.iter().map(|h| h.market_value).sum();

    let mut pe_numerator = 0.0_f64;
    let mut pe_denominator = 0.0_f64; // market value of PE>0 holdings
    let mut div_numerator = 0.0_f64;
    let mut pe_count: usize = 0;

    let mut items: Vec<HoldingValuation> = Vec::with_capacity(holdings.len());

    for h in holdings {
        let w = if total_mv > 0.0 {
            h.market_value / total_mv * 100.0
        } else {
            0.0
        };

        let quote = quotes.get(&h.symbol);
        let pe = quote.and_then(|q| q.pe_ttm).filter(|&p| p > 0.0);
        let div_yield = quote.and_then(|q| q.dividend_yield).filter(|&d| d > 0.0);

        if let Some(p) = pe {
            pe_numerator += h.market_value * p;
            pe_denominator += h.market_value;
            pe_count += 1;
        }
        if let Some(d) = div_yield {
            div_numerator += h.market_value * d;
        }

        items.push(HoldingValuation {
            symbol: h.symbol.clone(),
            name: h.name.clone(),
            market: h.market.clone(),
            weight: w,
            pe_ttm: pe,
            dividend_yield: div_yield,
            market_value: h.market_value,
        });
    }

    // Sort by weight descending
    items.sort_by(|a, b| b.market_value.partial_cmp(&a.market_value).unwrap_or(std::cmp::Ordering::Equal));

    let weighted_pe = if pe_denominator > 0.0 {
        Some(pe_numerator / pe_denominator)
    } else {
        None
    };

    let weighted_div = if total_mv > 0.0 {
        div_numerator / total_mv
    } else {
        0.0
    };

    let pe_coverage = if total_mv > 0.0 {
        pe_denominator / total_mv
    } else {
        0.0
    };

    PortfolioValuation {
        total_market_value: total_mv,
        weighted_pe_ttm: weighted_pe,
        weighted_dividend_yield: weighted_div,
        pe_coverage,
        pe_count,
        holdings: items,
    }
}
