use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptoContract {
    pub id: String,
    pub symbol: String,
    pub name: Option<String>,
    /// 资产类型: "crypto" 或 "tradfi"
    pub asset_type: String,
    /// 持仓方向: "long" 或 "short"
    pub position_type: String,
    /// 开仓价
    pub open_price: f64,
    /// 持仓数量（币/股）
    pub shares: f64,
    /// 杠杆倍数
    pub leverage: f64,
    /// 保证金（自动计算：open_price * shares / leverage + fee）
    pub margin: f64,
    /// 手续费
    pub fee: Option<f64>,
    /// 交易所
    pub exchange: Option<String>,
    /// 备注
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market_value: Option<f64>,
    /// 未实现盈亏
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pnl: Option<f64>,
    /// 收益率 (%)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pnl_pct: Option<f64>,
    /// 强平价格（估算，仅 crypto 有强平）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidation_price: Option<f64>,
    pub created_at: String,
    pub updated_at: String,
}

impl CryptoContract {
    pub fn from_db(
        id: String,
        symbol: String,
        name: Option<String>,
        asset_type: String,
        position_type: String,
        open_price: f64,
        shares: f64,
        leverage: f64,
        margin: f64,
        fee: Option<f64>,
        exchange: Option<String>,
        notes: Option<String>,
        created_at: String,
        updated_at: String,
    ) -> Self {
        // 估算强平价格
        // - crypto: 交易所维持保证金率约 0.5%
        // - tradfi: 美股融资维持保证金率 50%
        let liquidation_price = if leverage > 0.0 {
            let maintenance_margin_rate = if asset_type == "tradfi" {
                0.5
            } else {
                0.005
            };
            match position_type.as_str() {
                "long" => Some(open_price * (1.0 - (1.0 / leverage) + maintenance_margin_rate)),
                "short" => Some(open_price * (1.0 + (1.0 / leverage) + maintenance_margin_rate)),
                _ => None,
            }
        } else {
            None
        };

        Self {
            id,
            symbol,
            name,
            asset_type,
            position_type,
            open_price,
            shares,
            leverage,
            margin,
            fee,
            exchange,
            notes,
            current_price: None,
            market_value: None,
            pnl: None,
            pnl_pct: None,
            liquidation_price,
            created_at,
            updated_at,
        }
    }

    /// 计算平仓盈亏
    /// realized_pnl = (平仓价 - 开仓价) × 平仓数量 × direction - 开仓手续费 - 平仓手续费
    /// direction: long = +1, short = -1
    pub fn calculate_pnl(&self, close_price: f64, close_shares: f64, close_fee: f64) -> f64 {
        let direction = if self.position_type == "long" { 1.0 } else { -1.0 };
        let price_diff = (close_price - self.open_price) * direction;
        let gross_pnl = price_diff * close_shares;
        gross_pnl - self.fee.unwrap_or(0.0) - close_fee
    }
}
