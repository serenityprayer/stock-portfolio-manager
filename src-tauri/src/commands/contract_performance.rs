use tauri::State;
use crate::db::Database;
use crate::models::contract_performance::ContractPerformanceSummary;

/// 获取合约收益摘要（主命令）
#[tauri::command(rename_all = "camelCase")]
pub async fn get_contract_performance_summary(
    db: State<'_, Database>,
    start_date: String,
    end_date: String,
    force_refresh: Option<bool>,
) -> Result<ContractPerformanceSummary, String> {
    let start = chrono::NaiveDate::parse_from_str(&start_date, "%Y-%m-%d")
        .map_err(|e| format!("起始日期格式错误: {}", e))?;
    let end = chrono::NaiveDate::parse_from_str(&end_date, "%Y-%m-%d")
        .map_err(|e| format!("结束日期格式错误: {}", e))?;
    let force = force_refresh.unwrap_or(false);

    // 调用服务函数（使用完整路径）
    crate::services::contract_performance_service::get_contract_performance_summary(&db, start, end, force).await
}

/// 获取最早的合约日期（用于 ALL 时间范围）
#[tauri::command(rename_all = "camelCase")]
pub fn get_earliest_contract_date(
    db: State<'_, Database>,
) -> Result<String, String> {
    crate::services::contract_performance_service::get_earliest_contract_date(&db)
}

/// 一次性初始化：用现有合约数据回填历史净值快照
/// 会拉取 Gate.io / Yahoo Finance 历史价格，可能较慢
#[tauri::command(rename_all = "camelCase")]
pub async fn initialize_contract_performance(
    db: State<'_, Database>,
) -> Result<InitializeResult, String> {
    // 计算时间范围：从最早合约日期到昨天
    let earliest_str = crate::services::contract_performance_service::get_earliest_contract_date(&db)?;
    let start = chrono::NaiveDate::parse_from_str(&earliest_str, "%Y-%m-%d")
        .map_err(|e| format!("日期解析失败: {}", e))?;
    let end = chrono::Utc::now().naive_utc().date() - chrono::Duration::days(1);

    if start > end {
        return Ok(InitializeResult {
            inserted: 0,
            message: "暂无合约数据".to_string(),
        });
    }

    let count = crate::services::contract_performance_service::backfill_contract_snapshots(
        &db, start, end, true,
    ).await?;

    Ok(InitializeResult {
        inserted: count,
        message: format!("成功初始化 {} 条历史净值数据", count),
    })
}

#[derive(serde::Serialize)]
pub struct InitializeResult {
    pub inserted: i32,
    pub message: String,
}
