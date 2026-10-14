use std::collections::HashMap;
use rusqlite::Connection;
use chrono::{NaiveDate, Datelike};
use crate::db::Database;
use crate::models::contract_performance::*;

/// 自动补全合约历史净值快照
pub async fn backfill_contract_snapshots(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    force: bool,
) -> Result<i32, String> {
    // 1. 加载所有合约（活跃 + 已平仓）—— 同步操作，在 await 之前完成
    let all_contracts = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        load_all_contracts(&conn)?
    };

    if all_contracts.is_empty() {
        return Ok(0);
    }

    // force=true 时先删除已有数据再重新计算
    if force {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM daily_contract_values WHERE date >= ? AND date <= ?",
            rusqlite::params![
                start_date.format("%Y-%m-%d").to_string(),
                end_date.format("%Y-%m-%d").to_string(),
            ],
        ).map_err(|e| e.to_string())?;
    }

    // 2. 找出 daily_contract_values 中缺失的日期
    let missing_dates = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        find_missing_dates(&conn, start_date, end_date)?
    };

    if missing_dates.is_empty() {
        return Ok(0);
    }

    // 3. 拉取所有涉及标的的历史价格 —— 异步操作
    let mut price_map: HashMap<String, Vec<(NaiveDate, f64)>> = HashMap::new();

    for contract in &all_contracts {
        if price_map.contains_key(&contract.symbol) {
            continue;
        }

        let prices = if contract.asset_type == "crypto" {
            let quote_currency = "USDT";
            fetch_crypto_history(&contract.symbol, start_date, end_date, quote_currency).await?
        } else {
            let market = contract.market.as_deref().unwrap_or("US");
            crate::services::quote_service::fetch_stock_history_eastmoney(
                &contract.symbol,
                market,
                start_date,
                end_date,
            ).await?
        };

        price_map.insert(contract.symbol.clone(), prices);
    }

    // 4. 逐日计算净值并写入 —— 同步操作
    let mut count = 0;
    for date in missing_dates {
        let net_value = compute_net_value(&all_contracts, &price_map, date);

        match net_value {
            Ok(result) => {
                let conn = db.conn.lock().map_err(|e| e.to_string())?;
                conn.execute(
                    "INSERT OR REPLACE INTO daily_contract_values
                     (date, net_value, total_margin, unrealized_pnl, realized_pnl, total_pnl, return_rate)
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        date.format("%Y-%m-%d").to_string(),
                        result.net_value,
                        result.total_margin,
                        result.unrealized_pnl,
                        result.realized_pnl,
                        result.total_pnl,
                        result.return_rate,
                    ],
                ).map_err(|e| e.to_string())?;
                count += 1;
            }
            Err(e) => {
                eprintln!("[contract_performance] 跳过日期 {}: {}", date, e);
            }
        }
    }

    Ok(count)
}

/// 找出 daily_contract_values 中缺失的日期
fn find_missing_dates(
    db: &Connection,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<NaiveDate>, String> {
    let mut missing = Vec::new();
    let mut current = start_date;

    while current <= end_date {
        let date_str = current.format("%Y-%m-%d").to_string();
        let exists: i32 = db.query_row(
            "SELECT COUNT(*) FROM daily_contract_values WHERE date = ?",
            rusqlite::params![date_str],
            |row| row.get(0),
        ).map_err(|e| format!("查询快照失败: {}", e))?;

        if exists == 0 {
            missing.push(current);
        }

        current = current.succ_opt().unwrap();
    }

    Ok(missing)
}

/// 加载所有合约（活跃 + 已平仓）
fn load_all_contracts(db: &Connection) -> Result<Vec<ContractState>, String> {
    let mut contracts = Vec::new();

    // 1. 活跃合约
    let active_ids: std::collections::HashSet<String> = {
        let mut stmt = db.prepare("SELECT id FROM crypto_contract")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|e| e.to_string())?;
        rows.into_iter().collect()
    };

    let mut stmt = db.prepare(
        "SELECT id, symbol, asset_type, position_type, open_price, shares, leverage, margin, fee, exchange, created_at
         FROM crypto_contract"
    ).map_err(|e| e.to_string())?;

    let active_iter = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, f64>(4)?,
            row.get::<_, f64>(5)?,
            row.get::<_, f64>(6)?,
            row.get::<_, f64>(7)?,
            row.get::<_, Option<f64>>(8)?,
            row.get::<_, Option<String>>(9)?,
            row.get::<_, String>(10)?,
        ))
    }).map_err(|e| e.to_string())?;

    for item in active_iter {
        let (id, symbol, asset_type, position_type, open_price, shares, leverage, margin, _fee, exchange, created_at) =
            item.map_err(|e| e.to_string())?;

        let market = if asset_type == "tradfi" {
            Some(get_market_for_tradfi(db, &symbol, exchange.as_deref()))
        } else {
            None
        };

        contracts.push(ContractState {
            id,
            symbol,
            asset_type,
            position_type,
            avg_open_price: open_price,
            shares,
            leverage,
            margin,
            open_date: NaiveDate::parse_from_str(&created_at[..10], "%Y-%m-%d")
                .unwrap_or_else(|_| chrono::Utc::now().naive_utc().date()),
            close_date: None,
            realized_pnl: 0.0,
            market,
        });
    }

    // 2. 已平仓合约
    let mut closed_contracts: HashMap<String, ClosedContractSummary> = HashMap::new();

    let history_rows = db.prepare(
        "SELECT contract_id, symbol, asset_type, position_type, action_type, open_price,
                close_price, close_shares, add_price, add_shares, new_avg_price, new_total_shares,
                leverage, open_fee, close_fee, realized_pnl, closed_at
         FROM crypto_contract_history
         ORDER BY closed_at ASC"
    ).map_err(|e| e.to_string())?
    .query_map([], |row| {
        Ok(HistoryRow {
            contract_id: row.get(0)?,
            symbol: row.get(1)?,
            asset_type: row.get(2)?,
            position_type: row.get(3)?,
            action_type: row.get(4)?,
            open_price: row.get(5)?,
            close_price: row.get::<_, Option<f64>>(6)?,
            close_shares: row.get::<_, Option<f64>>(7)?,
            add_price: row.get::<_, Option<f64>>(8)?,
            add_shares: row.get::<_, Option<f64>>(9)?,
            new_avg_price: row.get::<_, Option<f64>>(10)?,
            new_total_shares: row.get::<_, Option<f64>>(11)?,
            leverage: row.get(12)?,
            open_fee: row.get::<_, Option<f64>>(13)?.unwrap_or(0.0),
            close_fee: row.get::<_, Option<f64>>(14)?.unwrap_or(0.0),
            realized_pnl: row.get::<_, Option<f64>>(15)?,
            closed_at: row.get(16)?,
        })
    }).map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())?;

    for row in history_rows {
        if active_ids.contains(&row.contract_id) {
            continue;
        }

        let summary = closed_contracts.entry(row.contract_id.clone()).or_insert_with(|| ClosedContractSummary {
            contract_id: row.contract_id.clone(),
            symbol: row.symbol.clone(),
            asset_type: row.asset_type.clone(),
            position_type: row.position_type.clone(),
            open_price: row.open_price,
            avg_open_price: row.open_price,
            total_shares: 0.0,
            remaining_shares: 0.0,
            total_margin: 0.0,
            total_fees: 0.0,
            leverage: row.leverage,
            realized_pnl: 0.0,
            close_date: None,
            open_date: None,
        });

        match row.action_type.as_str() {
            "open" => {
                let open_shares = row.close_shares.unwrap_or(0.0);
                summary.total_shares += open_shares;
                summary.remaining_shares += open_shares;
                summary.total_fees += row.open_fee;
                if summary.open_date.is_none() {
                    summary.open_date = Some(row.closed_at.clone());
                }
            }
            "add" => {
                let add_shares = row.add_shares.unwrap_or(0.0);
                summary.total_shares += add_shares;
                summary.remaining_shares += add_shares;
                summary.total_fees += row.open_fee;
                if let Some(new_avg) = row.new_avg_price {
                    summary.avg_open_price = new_avg;
                }
                if let Some(new_total) = row.new_total_shares {
                    summary.total_shares = new_total;
                }
            }
            "close" => {
                let close_shares = row.close_shares.unwrap_or(0.0);
                summary.remaining_shares -= close_shares;
                summary.total_fees += row.close_fee;
                if let Some(pnl) = row.realized_pnl {
                    summary.realized_pnl += pnl;
                }
                summary.close_date = Some(row.closed_at.clone());
            }
            _ => {}
        }
    }

    for (_, summary) in closed_contracts {
        let open_date = if let Some(date_str) = summary.open_date {
            NaiveDate::parse_from_str(&date_str[..10], "%Y-%m-%d")
                .unwrap_or_else(|_| chrono::Utc::now().naive_utc().date())
        } else {
            chrono::Utc::now().naive_utc().date()
        };

        let close_date = summary.close_date.as_ref()
            .and_then(|s| NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d").ok());

        contracts.push(ContractState {
            id: summary.contract_id,
            symbol: summary.symbol,
            asset_type: summary.asset_type,
            position_type: summary.position_type,
            avg_open_price: summary.avg_open_price,
            shares: 0.0,
            leverage: summary.leverage,
            margin: summary.total_margin,
            open_date,
            close_date,
            realized_pnl: summary.realized_pnl,
            market: None,
        });
    }

    Ok(contracts)
}

/// 为 tradfi 合约确定 market
fn get_market_for_tradfi(_db: &Connection, symbol: &str, exchange: Option<&str>) -> String {
    if symbol.ends_with(".SS") || symbol.ends_with(".SZ") || exchange == Some("SSE") {
        "CN".to_string()
    } else if symbol.ends_with(".HK") || exchange == Some("HKEX") {
        "HK".to_string()
    } else {
        "US".to_string()
    }
}

/// 计算指定日期的合约净值（纯计算，无数据库操作）
fn compute_net_value(
    contracts: &[ContractState],
    price_map: &HashMap<String, Vec<(NaiveDate, f64)>>,
    date: NaiveDate,
) -> Result<NetValueResult, String> {
    let mut total_margin = 0.0;
    let mut total_unrealized_pnl = 0.0;
    let mut total_realized_pnl = 0.0;

    for contract in contracts {
        if date < contract.open_date {
            continue;
        }
        if let Some(close_date) = contract.close_date {
            if date > close_date {
                total_realized_pnl += contract.realized_pnl;
                continue;
            }
        }

        total_margin += contract.margin;
        total_realized_pnl += contract.realized_pnl;

        if let Some(prices) = price_map.get(&contract.symbol) {
            let price = get_price_for_date(prices, date);

            if let Some(current_price) = price {
                let pnl = if contract.position_type == "long" {
                    (current_price - contract.avg_open_price) * contract.shares * contract.leverage
                } else {
                    (contract.avg_open_price - current_price) * contract.shares * contract.leverage
                };
                total_unrealized_pnl += pnl;
            }
        }
    }

    let total_pnl = total_unrealized_pnl + total_realized_pnl;
    let return_rate = if total_margin.abs() > 1e-8 {
        total_pnl / total_margin * 100.0
    } else {
        0.0
    };
    let net_value = total_margin + total_pnl;

    Ok(NetValueResult {
        net_value,
        total_margin,
        unrealized_pnl: total_unrealized_pnl,
        realized_pnl: total_realized_pnl,
        total_pnl,
        return_rate,
    })
}

struct NetValueResult {
    net_value: f64,
    total_margin: f64,
    unrealized_pnl: f64,
    realized_pnl: f64,
    total_pnl: f64,
    return_rate: f64,
}

/// 从价格列表中获取指定日期的价格（前向填充）
fn get_price_for_date(prices: &[(NaiveDate, f64)], date: NaiveDate) -> Option<f64> {
    for (d, p) in prices {
        if *d == date {
            return Some(*p);
        }
    }

    let mut best_price = None;
    let mut best_date = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();

    for (d, p) in prices {
        if *d < date && *d > best_date {
            best_date = *d;
            best_price = Some(*p);
        }
    }

    best_price
}

/// 获取加密货币历史日线价格（Gate.io K 线接口）
pub async fn fetch_crypto_history(
    symbol: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
    quote_currency: &str,
) -> Result<Vec<(NaiveDate, f64)>, String> {
    let pair = format!("{}_{}", symbol.to_uppercase(), quote_currency.to_uppercase());
    let start_ts = start_date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
    let end_ts = end_date.and_hms_opt(23, 59, 59).unwrap().and_utc().timestamp();

    let mut all_prices = Vec::new();
    let mut current_start = start_ts;

    for _ in 0..10 {
        let url = format!(
            "https://api.gateio.ws/api/v4/spot/candlesticks?currency_pair={}&interval=1d&from={}&to={}&limit=1000",
            pair, current_start, end_ts
        );

        let resp = reqwest::get(&url)
            .await
            .map_err(|e| format!("Gate.io API 请求失败: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Gate.io API 返回错误: {}", resp.status()));
        }

        let candlesticks: Vec<Vec<String>> = resp.json()
            .await
            .map_err(|e| format!("Gate.io API 响应解析失败: {}", e))?;

        if candlesticks.is_empty() {
            break;
        }

        for candle in &candlesticks {
            if candle.len() < 5 {
                continue;
            }

            let ts: i64 = candle[0].parse().map_err(|e| format!("时间戳解析失败: {}", e))?;
            let close_price: f64 = candle[2].parse().map_err(|e| format!("价格解析失败: {}", e))?;

            let date = chrono::DateTime::from_timestamp(ts, 0)
                .map(|dt| dt.naive_utc().date())
                .unwrap_or_else(|| chrono::Utc::now().naive_utc().date());

            all_prices.push((date, close_price));
        }

        let last_ts = candlesticks.last().unwrap()[0].parse::<i64>().unwrap();
        current_start = last_ts + 86400;

        if current_start >= end_ts {
            break;
        }
    }

    let mut unique_prices: HashMap<NaiveDate, f64> = HashMap::new();
    for (date, price) in all_prices {
        unique_prices.entry(date).or_insert(price);
    }

    let mut result: Vec<(NaiveDate, f64)> = unique_prices.into_iter().collect();
    result.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(result)
}

/// 计算风险指标
fn compute_risk_metrics(return_series: &[ContractReturnPoint], annualization_factor: f64) -> (f64, f64, f64) {
    if return_series.len() < 2 {
        return (0.0, 0.0, 0.0);
    }

    let mut peak = return_series[0].net_value;
    let mut max_drawdown = 0.0;
    for point in return_series {
        if point.net_value > peak {
            peak = point.net_value;
        }
        if peak > 0.0 {
            let drawdown = (peak - point.net_value) / peak * 100.0;
            if drawdown > max_drawdown {
                max_drawdown = drawdown;
            }
        }
    }

    let mut daily_returns = Vec::new();
    for i in 1..return_series.len() {
        let prev = return_series[i-1].net_value;
        let curr = return_series[i].net_value;
        if prev > 0.0 {
            daily_returns.push((curr - prev) / prev);
        }
    }

    if daily_returns.is_empty() {
        return (max_drawdown, 0.0, 0.0);
    }

    let mean_return = daily_returns.iter().sum::<f64>() / daily_returns.len() as f64;
    let variance = if daily_returns.len() > 1 {
        daily_returns.iter()
            .map(|r| (r - mean_return).powi(2))
            .sum::<f64>() / (daily_returns.len() - 1) as f64
    } else {
        0.0
    };
    let volatility = (variance.sqrt() * annualization_factor.sqrt()) * 100.0;

    let win_days = daily_returns.iter().filter(|r| **r > 0.0).count();
    let win_rate = win_days as f64 / daily_returns.len() as f64 * 100.0;

    (max_drawdown, volatility, win_rate)
}

/// 从 daily_contract_values 读取数据
pub fn read_contract_performance(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    annualization_factor: f64,
) -> Result<ContractPerformanceSummary, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    
    let mut stmt = conn.prepare(
        "SELECT date, net_value, total_margin, unrealized_pnl, realized_pnl, total_pnl, return_rate
         FROM daily_contract_values
         WHERE date >= ? AND date <= ?
         ORDER BY date ASC"
    ).map_err(|e| e.to_string())?;

    let rows = stmt.query_map(
        rusqlite::params![
            start_date.format("%Y-%m-%d").to_string(),
            end_date.format("%Y-%m-%d").to_string(),
        ],
        |row| {
            Ok(ContractReturnPoint {
                date: row.get(0)?,
                net_value: row.get(1)?,
                total_margin: row.get(2)?,
                unrealized_pnl: row.get(3)?,
                realized_pnl: row.get(4)?,
                total_pnl: row.get(5)?,
                return_rate: row.get(6)?,
            })
        },
    ).map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Ok(ContractPerformanceSummary {
            start_date: start_date.to_string(),
            end_date: end_date.to_string(),
            current_net_value: 0.0,
            total_margin: 0.0,
            total_pnl: 0.0,
            total_return: 0.0,
            max_drawdown: 0.0,
            volatility: 0.0,
            win_rate: 0.0,
            return_series: vec![],
        });
    }

    let last_point = rows.last().unwrap();
    let total_margin = last_point.total_margin;
    let total_pnl = last_point.total_pnl;
    let total_return = if total_margin.abs() > 1e-8 {
        total_pnl / total_margin * 100.0
    } else {
        0.0
    };

    let (max_drawdown, volatility, win_rate) = compute_risk_metrics(&rows, annualization_factor);

    Ok(ContractPerformanceSummary {
        start_date: start_date.to_string(),
        end_date: end_date.to_string(),
        current_net_value: last_point.net_value,
        total_margin,
        total_pnl,
        total_return,
        max_drawdown,
        volatility,
        win_rate,
        return_series: rows,
    })
}

/// 获取最早的合约日期
pub fn get_earliest_contract_date(db: &Database) -> Result<String, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    
    let result: Option<String> = conn.query_row(
        "SELECT MIN(date) FROM (
            SELECT MIN(created_at) as date FROM crypto_contract
            UNION ALL
            SELECT MIN(closed_at) as date FROM crypto_contract_history WHERE closed_at IS NOT NULL
        ) AS sub
        WHERE date IS NOT NULL",
        [],
        |row| row.get(0),
    ).map_err(|e| format!("查询最早合约日期失败: {}", e))?;

    match result {
        Some(date) => Ok(date[..10].to_string()),
        None => {
            let one_year_ago = chrono::Utc::now().naive_utc().date() - chrono::Duration::days(365);
            Ok(one_year_ago.format("%Y-%m-%d").to_string())
        }
    }
}

/// 获取合约收益摘要（主函数）
pub async fn get_contract_performance_summary(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    force_refresh: bool,
) -> Result<ContractPerformanceSummary, String> {
    // 自动 backfill
    let filled = backfill_contract_snapshots(db, start_date, end_date, force_refresh).await;
    if let Ok(n) = &filled {
        if *n > 0 {
            println!("[contract_performance] backfill {} missing dates", n);
        }
    }

    // 根据合约类型确定年化因子
    let (has_tradfi, has_crypto) = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let has_tradfi = conn.prepare("SELECT COUNT(*) FROM crypto_contract WHERE asset_type = 'tradfi'")
            .map_err(|e| e.to_string())?
            .query_row([], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        let has_crypto = conn.prepare("SELECT COUNT(*) FROM crypto_contract WHERE asset_type = 'crypto'")
            .map_err(|e| e.to_string())?
            .query_row([], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        Ok::<(i32, i32), String>((has_tradfi, has_crypto))
    }?;

    let annualization_factor = if has_tradfi > 0 && has_crypto == 0 {
        252.0
    } else {
        365.0
    };

    read_contract_performance(db, start_date, end_date, annualization_factor)
}
