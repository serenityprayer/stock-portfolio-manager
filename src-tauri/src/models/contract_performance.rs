use serde::{Deserialize, Serialize};
use chrono::NaiveDate;

/// 合约收益曲线数据点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractReturnPoint {
    pub date: String,              // "2024-01-01"
    pub net_value: f64,            // 当日净值 = total_margin + unrealized_pnl + realized_pnl
    pub total_margin: f64,         // 总保证金
    pub unrealized_pnl: f64,       // 未实现盈亏
    pub realized_pnl: f64,         // 已实现盈亏
    pub total_pnl: f64,            // 总盈亏 = unrealized_pnl + realized_pnl
    pub return_rate: f64,          // 收益率 (%) = total_pnl / total_margin * 100
}

/// 合约收益摘要（前端展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractPerformanceSummary {
    pub start_date: String,        // 起始日期
    pub end_date: String,          // 结束日期
    pub current_net_value: f64,    // 当前净值
    pub total_margin: f64,         // 总保证金
    pub total_pnl: f64,            // 总盈亏
    pub total_return: f64,         // 总收益率 (%)
    pub max_drawdown: f64,         // 最大回撤 (%)
    pub volatility: f64,           // 波动率 (%)
    pub win_rate: f64,             // 胜率 (%)
    pub return_series: Vec<ContractReturnPoint>,
}

/// 合约状态（用于 backfill 计算）
#[derive(Debug, Clone)]
pub struct ContractState {
    pub id: String,
    pub symbol: String,
    pub asset_type: String,        // "crypto" 或 "tradfi"
    pub position_type: String,     // "long" 或 "short"
    pub avg_open_price: f64,
    pub shares: f64,               // 当前持仓数量（活跃合约）或 0（已平仓）
    pub leverage: f64,
    pub margin: f64,               // 保证金 = open_price * shares / leverage + fee
    pub open_date: NaiveDate,
    pub close_date: Option<NaiveDate>,
    pub realized_pnl: f64,         // 该合约的已实现盈亏（平仓后固定）
    pub market: Option<String>,    // tradfi 合约需要：A股/港股/美股
}

/// 历史记录行（从 crypto_contract_history 表读取）
#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub contract_id: String,
    pub symbol: String,
    pub asset_type: String,
    pub position_type: String,
    pub action_type: String,       // "open" / "add" / "close"
    pub open_price: f64,
    pub close_price: Option<f64>,
    pub close_shares: Option<f64>,
    pub add_price: Option<f64>,
    pub add_shares: Option<f64>,
    pub new_avg_price: Option<f64>,
    pub new_total_shares: Option<f64>,
    pub leverage: f64,
    pub open_fee: f64,
    pub close_fee: f64,
    pub realized_pnl: Option<f64>,
    pub closed_at: String,         // ISO 8601 时间戳
}

/// 已平仓合约摘要（用于重建生命周期）
#[derive(Debug, Clone)]
pub struct ClosedContractSummary {
    pub contract_id: String,
    pub symbol: String,
    pub asset_type: String,
    pub position_type: String,
    pub open_price: f64,
    pub avg_open_price: f64,
    pub total_shares: f64,
    pub remaining_shares: f64,
    pub total_margin: f64,
    pub total_fees: f64,
    pub leverage: f64,
    pub realized_pnl: f64,
    pub close_date: Option<String>,
    pub open_date: Option<String>,
}
