use serde::{Deserialize, Serialize};

/// Portfolio-level valuation summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioValuation {
    pub total_market_value: f64,
    /// Weighted average PE (TTM). Only PE > 0 holdings are included.
    pub weighted_pe_ttm: Option<f64>,
    /// Weighted average dividend yield (%).
    pub weighted_dividend_yield: f64,
    /// Fraction of total market value that participates in PE calculation (PE > 0).
    pub pe_coverage: f64,
    /// Number of holdings with PE > 0.
    pub pe_count: usize,
    /// Per-holding breakdown.
    pub holdings: Vec<HoldingValuation>,
}

/// Per-holding valuation metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoldingValuation {
    pub symbol: String,
    pub name: String,
    pub market: String,
    /// Portfolio weight (% of total market value).
    pub weight: f64,
    pub pe_ttm: Option<f64>,
    pub dividend_yield: Option<f64>,
    pub market_value: f64,
}
