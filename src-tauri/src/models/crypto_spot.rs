use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptoSpot {
    pub id: String,
    pub symbol: String,
    pub name: Option<String>,
    pub buy_price: f64,
    pub shares: f64,
    pub fee: Option<f64>,
    pub exchange: Option<String>,
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pnl: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pnl_pct: Option<f64>,
    pub created_at: String,
    pub updated_at: String,
}

impl CryptoSpot {
    pub fn from_db(
        id: String,
        symbol: String,
        name: Option<String>,
        buy_price: f64,
        shares: f64,
        fee: Option<f64>,
        exchange: Option<String>,
        notes: Option<String>,
        created_at: String,
        updated_at: String,
    ) -> Self {
        Self {
            id,
            symbol,
            name,
            buy_price,
            shares,
            fee,
            exchange,
            notes,
            current_price: None,
            market_value: None,
            pnl: None,
            pnl_pct: None,
            created_at,
            updated_at,
        }
    }
}
