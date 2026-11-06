//! P3 — dialectic advisor data contracts.
//!
//! The advisor challenges assumptions; it never issues buy/sell instructions.
//! Every claim must carry a citation pointing at a LOCAL, traceable source.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    /// investment_memos | transactions | risk_events | trade_reviews | watchlist | risk_rules
    pub source: String,
    pub ref_id: Option<String>,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Challenge {
    pub claim: String,
    pub basis: String,
    pub citation: Option<Citation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrincipleConflict {
    pub principle: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeAnalysis {
    /// challenge | neutral
    pub verdict: String,
    pub challenges: Vec<Challenge>,
    pub principle_conflicts: Vec<PrincipleConflict>,
    pub risk_notes: Vec<String>,
    pub suggestions: Vec<String>,
    pub citations: Vec<Citation>,
    /// Set when the model did not return valid JSON — the raw reply is kept
    /// so nothing is silently lost.
    pub raw: Option<String>,
    /// True when no API key is configured and only local evidence is returned.
    pub degraded: bool,
    pub transaction_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorReply {
    pub reply: String,
    pub citations: Vec<Citation>,
    pub used_sources: Vec<String>,
    pub degraded: bool,
}

/// A retrieved local snippet with its relevance score.
#[derive(Debug, Clone)]
pub struct LocalEvidence {
    pub source: String,
    pub ref_id: String,
    pub excerpt: String,
    pub score: f64,
}
