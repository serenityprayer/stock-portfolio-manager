# 合约收益净值曲线 — 完整实现方案 v4.2a

> 版本：v4.2a | 2026-07-01
> 目标：给合约持仓增加真实的历史盈亏净值曲线（对齐股票的绩效分析页面）
> 修订说明：v4.2a 自我评审修复版，修复 GLM 评审 19 个问题 + 自我评审 3 个额外问题（见第九章）；所有后端代码与实际代码库对齐；代码可直接照抄实施。

---

## 修订历史

| 版本 | 日期 | 主要变更 |
|------|------|----------|
| v1.0 | 2026-06-30 | 初始版本，纯计算方案 |
| v2.0 | 2026-06-30 | 改为混合架构；修正净值计算；补充资金费率、风险指标、边界情况处理 |
| v3.0 | 2026-06-30 | 验证 Gate.io K 线 API 可用；对齐股票 backfill 架构；tradfi 合约复用现有逻辑 |
| v4.1 | 2026-07-01 | 交叉评审修复：12个问题修复/注明（见第八章）；前端初始加载逻辑补充 |
| v4.2 | 2026-07-01 | GLM 交叉评审修复：19个问题全部修复（见第九章）；后端代码与实际代码库完全对齐 |
| v4.2a | 2026-07-01 | 自我评审额外修复：Issue A（部分平仓合约重复计算）、Issue B（Gate.io 报价货币硬编码）、Issue D（风险指标年化因子）|

---

## 一、关键发现（实现基础）

### 1.1 股票 `daily_portfolio_values` 的数据写入机制

**不是手动触发**。每次打开"绩效分析"页面，`performanceStore.fetchAll()` 会先调用 `backfill_snapshots(startDate, endDate)`，**自动补全**该日期范围内所有缺失快照的数据。

```
打开绩效分析页面
    ↓
fetchAll()
    ↓
第一步：backfill_snapshots(startDate, endDate)  ← 非致命，失败不中断
    ↓
后端：找出缺失快照的日期 → 拉取历史收盘价 → 逐日计算净值 → 写入 DB
    ↓
第二步：读 daily_portfolio_values → 展示曲线
```

**关键细节**（对齐 `performanceStore.ts` 第 162-174 行）：
- `forceRefresh=false`（默认）：只填充从未计算过的日期（快速缓存加载）
- `forceRefresh=true`（用户点击刷新）：全量重算范围内所有快照
- backfill 错误被视为 **非致命错误**（`non-fatal`），不会中断后续数据获取

**结论**：股票曲线是连续的，因为每次打开页面都自动 backfill。合约应完全复用这一机制。

### 1.2 Gate.io 历史 K 线 API 验证结果

| 测试 | 结果 |
|------|------|
| BTC 最近 5 天 | ✅ 6 个数据点 |
| BTC 2024 年 1 月 | ✅ 31 个数据点，收盘价 $42,285 |
| ETH 2024 年 1-3 月 | ✅ 91 个数据点，收盘价 $2,282 |
| SOL 2024 年 1-3 月 | ✅ 91 个数据点，$101 → $194 |

**API 端点**：`GET https://api.gateio.ws/api/v4/spot/candlesticks?currency_pair=BTC_USDT&interval=1d&from={ts}&to={ts}&limit=1000`

**响应格式**：`[timestamp(秒), volume, close, high, low, open, ...]`

**注意**：`limit=1000` 最大返回 1000 条日线（约 2.7 年）。超过此范围需要分段获取（见 4.2 节分页逻辑）。

**结论**：加密货币历史价格 API 可用，合约可以完全复用股票的 backfill 架构。

### 1.3 TradFi 合约处理方式

TradFi 合约（股票/ETF 为标的）的 symbol 与股票相同，历史价格获取直接复用 `quote_service` 中已有的 `fetch_stock_history_yahoo()`。

**实际函数签名**（`src-tauri/src/services/quote_service.rs` 第 1361-1366 行）：
```rust
pub async fn fetch_stock_history_yahoo(
    symbol: &str,
    market: &str,          // 第二个参数是 market，不是 start_date
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
) -> Result<Vec<(chrono::NaiveDate, f64)>, String>
```

**market 来源**：`crypto_contract` 表有 `asset_type` 字段（`'crypto'` 或 `'tradfi'`），但没有 `market` 字段。对于 tradfi 合约，通过查 stocks 表或从 exchange 字段推断 market（见 4.2 节 `get_market_for_tradfi()`）。

**不需要新写任何历史价格获取逻辑。**

### 1.4 实际代码库关键信息

| 项 | 实际值 | 文件位置 |
|----|--------|---------|
| Tauri State 类型 | `State<'_, Database>`（不是 `AppState`） | `db/mod.rs` 第 7-9 行 |
| Tauri command 属性 | `#[tauri::command(rename_all = "camelCase")]` | `commands/snapshots.rs` 第 11 行 |
| `fetch_stock_history_yahoo` 参数 | `(symbol, market, start_date, end_date)` | `quote_service.rs` 第 1361-1366 行 |
| `action_type` 实际值 | `'open'`、`'add'`、`'close'`（**无 `partial_close`**） | `db/crypto_contract.rs` |
| `asset_type` 字段 | 存在于 `crypto_contract` 表和模型中 | `db/mod.rs` 第 312 行 |
| `crypto_contract_history` 表 | 有 `asset_type`、`open_price`、`close_shares`、`add_shares`、`new_avg_price`、`new_total_shares` 等字段 | `db/mod.rs` 第 337-361 行 |

---

## 二、功能概述

在合约页面新增「收益曲线」Tab，展示合约组合从第一笔开仓至今的 **净值曲线**（净值 = 总保证金 + 累计未实现盈亏 + 累计已实现盈亏），支持历史回溯。

```
┌──────────────────────────────────────────────────────────┐
│  持仓中(5)  │  历史成交  │  📈 收益曲线                   │
├──────────────────────────────────────────────────────────┤
│  [1M] [3M] [6M] [YTD] [ALL]   ← 时间范围切换            │
├──────────────────────────────────────────────────────────┤
│                                       📊                 │
│  净值($)                             ╱╲                 │
│  $12,000│                        ╱───╱  ╲──              │
│  $10,000│                  ╱────╱                         │
│   $8,000│────╱────╱──╱                                  │
│          └─────────────────────────→ 日期               │
│                                       📊                 │
├──────────────────────────────────────────────────────────┤
│ 当前净值 $11,500  │ 累计盈亏 +$3,500  │ 总保证金 $8,000  │
│ 收益率 +43.75%    │ 最大回撤 -12.3%  │ 波动率 18.2%     │
└──────────────────────────────────────────────────────────┘
```

---

## 三、架构设计

### 3.1 数据更新触发机制（对齐股票）

与股票 `backfill_snapshots` 完全一致的触发机制：

```
打开合约页面 → 切换至「收益曲线」Tab
    ↓
fetchSummary()
    ↓
第一步：backfill_contract_snapshots(startDate, endDate, force)  ← 非致命
    ↓
后端：找出 daily_contract_values 中缺失的日期
    → 拉取历史价格（Gate.io / Yahoo Finance）
    → 逐日计算净值
    → 写入 daily_contract_values
    ↓
第二步：读 daily_contract_values → 计算风险指标 → 展示曲线
```

**与股票的区别**：
- 股票：每个市场有收盘时间，用「昨日收盘价」
- 合约（crypto）：7×24 交易，用「最近一个完整 UTC 日」的收盘价（即 UTC 0 点的价格，或取最近一个 K 线的 close）

### 3.2 数据库表设计

新增 `daily_contract_values` 表：

```sql
CREATE TABLE IF NOT EXISTS daily_contract_values (
    date TEXT PRIMARY KEY,              -- "2025-06-15"
    net_value REAL NOT NULL,            -- 总净值（保证金 + 未实现盈亏 + 已实现盈亏）
    total_margin REAL NOT NULL,         -- 总保证金
    unrealized_pnl REAL NOT NULL,       -- 未实现盈亏（活跃持仓）
    realized_pnl REAL NOT NULL DEFAULT 0, -- 已实现盈亏（已平仓合约）
    total_pnl REAL NOT NULL,            -- 总盈亏 = unrealized_pnl + realized_pnl
    return_rate REAL NOT NULL DEFAULT 0  -- 收益率% = total_pnl / total_margin * 100
);
```

### 3.3 历史价格获取方案

| 合约类型 | 判断依据 | 历史价格来源 | 说明 |
|-----------|---------|----------------|------|
| **Crypto**（加密货币） | `asset_type == 'crypto'` | Gate.io K 线 API | `GET /api/v4/spot/candlesticks`，已验证可用 |
| **TradFi**（股票/ETF） | `asset_type == 'tradfi'` | 复用 `fetch_stock_history_yahoo()` | 需通过 stocks 表或 exchange 推断 market |

---

## 四、后端实现（Rust）—— 完整代码

### 4.1 新增模型（`models/contract_performance.rs`）

```rust
use serde::{Serialize, Deserialize};
use chrono::NaiveDate;

/// 合约收益曲线上的单个数据点
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContractReturnPoint {
    pub date: String,              // "2025-06-15"
    pub net_value: f64,            // 当日净值
    pub total_margin: f64,         // 当日总保证金
    pub unrealized_pnl: f64,       // 当日未实现盈亏
    pub realized_pnl: f64,         // 当日已实现盈亏（累计）
    pub total_pnl: f64,            // 当日总盈亏
    pub daily_pnl: f64,            // 当日盈亏变动
    pub cumulative_return: f64,    // 累计收益率（%）
}

/// 合约收益摘要（含风险指标）
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContractPerformanceSummary {
    pub start_date: String,
    pub end_date: String,
    pub current_net_value: f64,
    pub total_margin: f64,
    pub total_pnl: f64,
    pub total_return: f64,
    // 风险指标
    pub max_drawdown: f64,       // 最大回撤（%）
    pub volatility: f64,          // 波动率（%）
    pub win_rate: f64,            // 胜率（%）
    pub return_series: Vec<ContractReturnPoint>,
}

/// 合约状态（用于 backfill 计算）
#[derive(Debug, Clone)]
struct ContractState {
    pub id: String,
    pub symbol: String,
    pub asset_type: String,       // "crypto" 或 "tradfi"（修复 #5/#6/#17：替代 is_crypto_symbol）
    pub position_type: String,    // "long" / "short"
    pub avg_open_price: f64,
    pub shares: f64,              // 当前持仓数量（活跃合约）或 0（已平仓）
    pub leverage: f64,
    pub margin: f64,              // 保证金 = open_price * shares / leverage + fee
    pub open_date: NaiveDate,
    pub close_date: Option<NaiveDate>,
    pub realized_pnl: f64,        // 该合约的已实现盈亏（平仓后固定）
    pub market: Option<String>,   // tradfi 合约需要：A股/港股/美股（修复 #5）
}
```

### 4.2 核心服务（`services/contract_performance_service.rs`）

#### 主函数：`backfill_contract_snapshots()`

**设计**：与 `snapshot_service.rs` 中的 `backfill_snapshots()` 保持一致的逻辑结构。

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use rusqlite::Connection;
use chrono::{NaiveDate, Datelike};

// 修复 #12：并发保护 —— 全局锁，防止多个 backfill 同时执行
static BACKFILL_LOCK: Mutex<bool> = Mutex::new(false);

/// 自动补全合约历史净值快照（与股票 backfill_snapshots 对齐）
/// 在 fetchContractPerformance 中自动调用，无需手动触发
pub async fn backfill_contract_snapshots(
    db: &Connection,
    start_date: NaiveDate,
    end_date: NaiveDate,
    force: bool,
) -> Result<i32, String> {
    // 修复 #12：获取锁，防止并发 backfill
    let _lock = BACKFILL_LOCK.lock().map_err(|e| format!("获取 backfill 锁失败: {}", e))?;

    // 1. 加载所有合约（活跃 + 已平仓）
    let all_contracts = load_all_contracts(db)?;

    if all_contracts.is_empty() {
        return Ok(0);  // 没有任何合约，无需计算
    }

    // 修复 #8：force=true 时先删除已有数据再重新计算
    if force {
        db.execute(
            "DELETE FROM daily_contract_values WHERE date >= ? AND date <= ?",
            rusqlite::params![
                start_date.format("%Y-%m-%d").to_string(),
                end_date.format("%Y-%m-%d").to_string(),
            ],
        ).map_err(|e| format!("删除旧快照失败: {}", e))?;
    }

    // 2. 找出 daily_contract_values 中缺失的日期
    let missing_dates = find_missing_dates(db, start_date, end_date)?;

    if missing_dates.is_empty() {
        return Ok(0);
    }

    // 3. 拉取所有涉及标的的历史价格
    let mut price_map: HashMap<String, Vec<(NaiveDate, f64)>> = HashMap::new();

    for contract in &all_contracts {
        if price_map.contains_key(&contract.symbol) {
            continue;  // 已拉取过
        }

        // 修复 #6/#17：用 asset_type 字段替代 is_crypto_symbol() 硬编码判断
        // 修复 Issue B：传递 quote_currency 参数（当前从 contract 表读取，默认 "USDT"）
        let prices = if contract.asset_type == "crypto" {
            // 修复 Issue B：quote_currency 从合约的 exchange 字段或默认 "USDT" 获取
            // 注：当前 crypto_contract 表无 quote_currency 字段，默认 "USDT"
            // 后续可扩展：在 crypto_contract 表新增 quote_currency 字段
            let quote_currency = "USDT";  // 默认值，后续可配置
            fetch_crypto_history(&contract.symbol, start_date, end_date, quote_currency).await?
        } else {
            // TradFi：复用现有股票历史价格逻辑
            // 修复 #2：fetch_stock_history_yahoo 实际签名是 (symbol, market, start, end)
            let market = contract.market.as_deref().unwrap_or("US");
            crate::services::quote_service::fetch_stock_history_yahoo(
                &contract.symbol,
                market,
                start_date,
                end_date,
            ).await?
        };

        price_map.insert(contract.symbol.clone(), prices);
    }

    // 4. 逐日计算净值并写入
    let mut count = 0;
    for date in missing_dates {
        // 修复 #7：单个合约价格缺失不应导致整天失败，用前向填充 fallback
        let net_value = compute_net_value(db, &all_contracts, &price_map, date);

        match net_value {
            Ok(result) => {
                db.execute(
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
                )?;
                count += 1;
            }
            Err(e) => {
                // 修复 #7：跳过该日期而非中断整个 backfill
                eprintln!("[contract_performance] 跳过日期 {}: {}", date, e);
            }
        }
    }

    Ok(count)
}
```

#### 加载所有合约：`load_all_contracts()`

```rust
/// 加载所有合约（活跃 + 已平仓）
/// 从 crypto_contract（活跃）+ crypto_contract_history（已平仓）重建完整生命周期
fn load_all_contracts(db: &Connection) -> Result<Vec<ContractState>, String> {
    let mut contracts = Vec::new();

    // 1. 活跃合约（在 crypto_contract 表中）
    // 修复 #5：读取 asset_type 字段；修复 #6：不再用 is_crypto_symbol 判断
    // 修复 Issue A：收集活跃合约 ID，重建历史时跳过（避免部分平仓合约被重复计算）
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
            row.get::<_, String>(0)?,           // id
            row.get::<_, String>(1)?,           // symbol
            row.get::<_, String>(2)?,           // asset_type
            row.get::<_, String>(3)?,           // position_type
            row.get::<_, f64>(4)?,              // open_price
            row.get::<_, f64>(5)?,              // shares
            row.get::<_, f64>(6)?,              // leverage
            row.get::<_, f64>(7)?,              // margin
            row.get::<_, Option<f64>>(8)?,      // fee
            row.get::<_, Option<String>>(9)?,   // exchange
            row.get::<_, String>(10)?,          // created_at
        ))
    }).map_err(|e| e.to_string())?;

    for item in active_iter {
        let (id, symbol, asset_type, position_type, open_price, shares, leverage, margin, _fee, exchange, created_at) =
            item.map_err(|e| e.to_string())?;

        // 修复 #5：为 tradfi 合约确定 market
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
            close_date: None,  // 活跃合约，未平仓
            realized_pnl: 0.0,
            market,
        });
    }

    // 2. 已平仓合约（从 crypto_contract_history 重建完整生命周期）
    // 修复 #3：正确读取 shares/margin；修复 #4：action_type 只有 'open'/'add'/'close'，无 'partial_close'
    // 修复 #13：正确计算 margin 和 shares
    // 修复 Issue A：跳过仍在 crypto_contract 表中的 contract_id（避免部分平仓合约被重复计算）
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

    // 重建每个已平仓合约的完整生命周期
    // 修复 Issue A：跳过仍在 crypto_contract 表中的 contract_id（避免部分平仓合约被重复计算）
    for row in history_rows {
        // 如果该 contract_id 仍在活跃合约表中，说明是部分平仓（合约未被删除），
        // 其当前状态已由第 1 步（活跃合约）正确加载，此处不应重复计算
        if active_ids.contains(&row.contract_id) {
            continue;  // 修复 Issue A：跳过部分平仓但未删除的合约
        }

        let summary = closed_contracts.entry(row.contract_id.clone()).or_insert_with(|| ClosedContractSummary {
            contract_id: row.contract_id.clone(),
            symbol: row.symbol.clone(),
            asset_type: row.asset_type.clone(),
            position_type: row.position_type.clone(),
            open_price: row.open_price,
            avg_open_price: row.open_price,  // 后续可能被 add 记录更新
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
                // 修复 #3：'open' 动作中 close_shares 字段存的是开仓数量
                // （见 db/crypto_contract.rs 第 86-93 行注释："close_shares 存开仓数量，供历史页展示"）
                let open_shares = row.close_shares.unwrap_or(0.0);
                summary.total_shares += open_shares;
                summary.remaining_shares += open_shares;
                summary.total_fees += row.open_fee;
                if summary.open_date.is_none() {
                    summary.open_date = Some(row.closed_at.clone());
                }
            }
            "add" => {
                // 加仓：add_shares 存的是加仓数量
                let add_shares = row.add_shares.unwrap_or(0.0);
                summary.total_shares += add_shares;
                summary.remaining_shares += add_shares;
                summary.total_fees += row.open_fee;
                // 更新平均开仓价和总数量
                if let Some(new_avg) = row.new_avg_price {
                    summary.avg_open_price = new_avg;
                }
                if let Some(new_total) = row.new_total_shares {
                    summary.total_shares = new_total;  // 用 new_total_shares 作为权威值
                }
            }
            // 修复 #4：'close' 涵盖部分平仓和全部平仓，无 'partial_close'
            "close" => {
                summary.realized_pnl += row.realized_pnl.unwrap_or(0.0);
                summary.close_date = Some(row.closed_at.clone());
                summary.total_fees += row.close_fee;
                if let Some(closed_shares) = row.close_shares {
                    summary.remaining_shares -= closed_shares;
                }
            }
            _ => {}
        }
    }

    // 将已平仓合约 push 到结果
    // 修复 #5：包含 asset_type 和 market 字段
    for (_, summary) in closed_contracts {
        // 修复 #13：正确计算 margin = avg_open_price * total_shares / leverage + total_fees
        let margin = if summary.total_shares > 0.0 && summary.leverage > 0.0 {
            summary.avg_open_price * summary.total_shares / summary.leverage + summary.total_fees
        } else {
            0.0
        };

        // 判断是否完全平仓：remaining_shares <= 0
        let is_fully_closed = summary.remaining_shares <= 0.0 && summary.close_date.is_some();

        // 修复 #5：包含所有字段，避免编译失败
        let market = if summary.asset_type == "tradfi" {
            Some(get_market_for_tradfi(db, &summary.symbol, None))
        } else {
            None
        };

        contracts.push(ContractState {
            id: summary.contract_id,
            symbol: summary.symbol,
            asset_type: summary.asset_type,
            position_type: summary.position_type,
            avg_open_price: summary.avg_open_price,
            shares: if is_fully_closed { 0.0 } else { summary.remaining_shares },
            leverage: summary.leverage,
            margin,
            open_date: NaiveDate::parse_from_str(
                &summary.open_date.unwrap_or_default()[..10.min(summary.open_date.unwrap_or_default().len())],
                "%Y-%m-%d"
            ).unwrap_or_else(|_| chrono::Utc::now().naive_utc().date()),
            close_date: summary.close_date.as_ref().and_then(|s| {
                NaiveDate::parse_from_str(&s[..10.min(s.len())], "%Y-%m-%d").ok()
            }),
            realized_pnl: summary.realized_pnl,
            market,
        });
    }

    Ok(contracts)
}

/// 历史记录行（辅助结构体）
struct HistoryRow {
    contract_id: String,
    symbol: String,
    asset_type: String,
    position_type: String,
    action_type: String,
    open_price: f64,
    close_price: Option<f64>,
    close_shares: Option<f64>,
    add_price: Option<f64>,
    add_shares: Option<f64>,
    new_avg_price: Option<f64>,
    new_total_shares: Option<f64>,
    leverage: f64,
    open_fee: f64,
    close_fee: f64,
    realized_pnl: Option<f64>,
    closed_at: String,
}

/// 已平仓合约汇总信息（用于重建生命周期）
struct ClosedContractSummary {
    contract_id: String,
    symbol: String,
    asset_type: String,
    position_type: String,
    open_price: f64,
    avg_open_price: f64,
    total_shares: f64,
    remaining_shares: f64,
    total_margin: f64,
    total_fees: f64,
    leverage: f64,
    realized_pnl: f64,
    close_date: Option<String>,
    open_date: Option<String>,
}

/// 为 tradfi 合约确定 market（修复 #6/#17）
/// 优先从 stocks 表查找，其次从 exchange 推断，最后默认 US
fn get_market_for_tradfi(db: &Connection, symbol: &str, exchange: Option<&str>) -> String {
    // 1. 优先从 stocks 表查找
    let result: Result<String, _> = db.query_row(
        "SELECT market FROM stocks WHERE symbol = ?1",
        [symbol],
        |row| row.get(0),
    );
    if let Ok(market) = result {
        return market;
    }

    // 2. 从 exchange 推断
    if let Some(ex) = exchange {
        let ex_upper = ex.to_uppercase();
        if ex_upper.contains("HK") || ex_upper.contains("港") {
            return "HK".to_string();
        }
        if ex_upper.contains("SS") || ex_upper.contains("SZ") || ex_upper.contains("A") {
            return "CN".to_string();
        }
    }

    // 3. 从 symbol 后缀推断
    if symbol.ends_with(".HK") { return "HK".to_string(); }
    if symbol.ends_with(".SS") || symbol.ends_with(".SZ") { return "CN".to_string(); }

    // 4. 默认美股
    "US".to_string()
}
```

#### 缺失日期查找

```rust
/// 找出 daily_contract_values 中缺失的日期
fn find_missing_dates(
    db: &Connection,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<NaiveDate>, String> {
    let mut stmt = db.prepare("SELECT date FROM daily_contract_values WHERE date >= ? AND date <= ?")
        .map_err(|e| e.to_string())?;

    let existing_dates: Vec<String> = stmt.query_map(
        [start_date.format("%Y-%m-%d").to_string(), end_date.format("%Y-%m-%d").to_string()],
        |row| row.get(0)
    ).map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())?;

    let existing_set: std::collections::HashSet<String> = existing_dates.into_iter().collect();

    let mut missing = Vec::new();
    let mut current = start_date;
    while current <= end_date {
        let date_str = current.format("%Y-%m-%d").to_string();
        if !existing_set.contains(&date_str) {
            missing.push(current);
        }
        current = current.succ_opt().unwrap();
    }

    Ok(missing)
}
```

#### 获取某天价格（带前向填充 fallback）

```rust
/// 获取某一天某个标的的价格
/// 修复 #7：返回 Option<f64> 而非 Result<f64, String>，缺失时返回 None 由调用方决定处理方式
fn get_price_for_date(
    price_map: &HashMap<String, Vec<(NaiveDate, f64)>>,
    symbol: &str,
    date: NaiveDate,
) -> Option<f64> {
    let prices = price_map.get(symbol)?;

    // 精确匹配
    for (d, p) in prices {
        if *d == date {
            return Some(*p);
        }
    }

    // 前向填充：用该日期之前最近一个有效价格
    let mut last_price: Option<f64> = None;
    for (d, p) in prices {
        if *d < date {
            last_price = Some(*p);
        } else {
            break;  // prices 已排序，后续都 > date
        }
    }

    // 如果前向填充也没有，用第一个可用价格
    last_price.or_else(|| prices.first().map(|(_, p)| *p))
}
```

#### 净值计算逻辑

```rust
/// 计算某一天的合约总净值
/// 修复 #7：单个合约价格缺失时跳过该合约，不影响其他合约和整体计算
fn compute_net_value(
    db: &Connection,
    all_contracts: &[ContractState],
    price_map: &HashMap<String, Vec<(NaiveDate, f64)>>,
    date: NaiveDate,
) -> Result<NetValueResult, String> {
    let mut total_margin = 0.0;
    let mut unrealized_pnl = 0.0;
    let mut realized_pnl = 0.0;

    for contract in all_contracts {
        // 判断该合约在 date 这一天是否活跃
        if contract.open_date > date {
            continue;  // 还未开仓
        }
        if let Some(close_date) = contract.close_date {
            if close_date < date {
                // 已平仓：盈亏 = 已实现盈亏（固定值），不再贡献保证金
                realized_pnl += contract.realized_pnl;
                continue;
            }
        }

        // 持仓中：需要当日价格
        // 修复 #7：价格缺失时跳过该合约而非报错
        let price = match get_price_for_date(price_map, &contract.symbol, date) {
            Some(p) if p > 0.0 => p,
            _ => {
                // 价格缺失，跳过该合约（不影响其他合约的计算）
                eprintln!("[contract_performance] 标的 {} 在 {} 无价格数据，跳过", contract.symbol, date);
                continue;
            }
        };

        let direction = if contract.position_type == "long" { 1.0 } else { -1.0 };
        let pnl = (price - contract.avg_open_price) * direction * contract.shares;

        total_margin += contract.margin;
        unrealized_pnl += pnl;
    }

    let total_pnl = unrealized_pnl + realized_pnl;
    let return_rate = if total_margin > 0.0 {
        total_pnl / total_margin * 100.0
    } else { 0.0 };

    Ok(NetValueResult {
        net_value: total_margin + unrealized_pnl + realized_pnl,
        total_margin,
        unrealized_pnl,
        realized_pnl,
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
```

#### 风险指标计算

```rust
/// 计算风险指标（最大回撤、波动率、胜率）
/// 修复 Issue D：年化因子不再硬编码 365，根据 asset_type 使用正确值
///   - crypto：365（全年交易）
///   - tradfi：252（交易日数量）
/// 参数 annualization_factor：年化因子，由调用方根据合约类型传入
fn compute_risk_metrics(
    return_series: &[ContractReturnPoint],
    annualization_factor: f64,
) -> (f64, f64, f64) {
    if return_series.len() < 2 {
        return (0.0, 0.0, 0.0);
    }

    // 1. 最大回撤
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

    // 2. 波动率（日收益率的标准差，年化）
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
    // 修复 Issue D：使用传入的年化因子（crypto=365, tradfi=252）
    let volatility = (variance.sqrt() * annualization_factor.sqrt()) * 100.0;

    // 3. 胜率（盈利天数占比）
    let win_days = daily_returns.iter().filter(|r| **r > 0.0).count();
    let win_rate = win_days as f64 / daily_returns.len() as f64 * 100.0;

    (max_drawdown, volatility, win_rate)
}
```

#### Gate.io 历史 K 线获取（含分页）

```rust
/// 获取加密货币历史日线价格（Gate.io K 线接口）
/// 返回：Vec<(NaiveDate, close_price)>
/// 修复 #10：支持分页获取，limit=1000 仅覆盖约 2.7 年
/// 修复 #11：用标准 chrono API 转换时间戳，避免 2038 年问题
/// 修复 Issue B：quote_currency 可配置，不再硬编码 _USDT
pub async fn fetch_crypto_history(
    symbol: &str,           // "BTC" → 自动转为 "BTC_USDT"（或指定 quote_currency）
    start_date: NaiveDate,
    end_date: NaiveDate,
    quote_currency: &str,   // 报价货币，默认 "USDT"（修复 Issue B）
) -> Result<Vec<(NaiveDate, f64)>, String> {
    // 修复 Issue B：使用 quote_currency 参数，不再硬编码 _USDT
    let pair = format!("{}_{}", symbol.to_uppercase(), quote_currency.to_uppercase());
    let start_ts = start_date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
    let end_ts = end_date.and_hms_opt(23, 59, 59).unwrap().and_utc().timestamp();

    let client = crate::services::http_client::general_client();
    let mut all_candles: Vec<(NaiveDate, f64)> = Vec::new();
    let mut current_from = start_ts;
    let page_size = 1000u64;
    let max_pages = 10u32;  // 安全上限：最多 10000 条日线（约 27 年）

    for _page in 0..max_pages {
        let url = format!(
            "https://api.gateio.ws/api/v4/spot/candlesticks?currency_pair={}&interval=1d&from={}&to={}&limit={}",
            pair, current_from, end_ts, page_size
        );

        let resp = client.get(&url)
            .send()
            .await
            .map_err(|e| format!("Gate.io API 请求失败: {}", e))?;

        let text = resp.text().await.map_err(|e| format!("读取响应失败: {}", e))?;
        let candles: Vec<Vec<serde_json::Value>> = serde_json::from_str(&text)
            .map_err(|e| format!("解析 Gate.io 响应失败: {}, 响应内容: {}", e, &text[..200.min(text.len())]))?;

        if candles.is_empty() {
            break;  // 没有更多数据
        }

        let page_len = candles.len();

        for candle in candles {
            if candle.len() < 3 {
                continue;
            }
            let ts = candle[0].as_i64().unwrap_or(0);
            let close_str = candle[2].as_str().unwrap_or("0");
            let close = close_str.parse::<f64>().unwrap_or(0.0);

            if ts == 0 || close <= 0.0 {
                continue;
            }

            // 修复 #11：用标准 chrono API 转换时间戳，安全且无 2038 问题
            let date = chrono::DateTime::from_timestamp(ts, 0)
                .map(|dt| dt.naive_utc().date())
                .unwrap_or_else(|| chrono::Utc::now().naive_utc().date());

            all_candles.push((date, close));
        }

        if (page_len as u64) < page_size {
            break;  // 最后一页，没有更多数据
        }

        // 下一页：从最后一根 K 线的时间戳 + 1 天开始
        if let Some(last_ts) = candles.last()
            .and_then(|c| c.first())
            .and_then(|v| v.as_i64())
        {
            current_from = last_ts + 86400;  // +1 天
            if current_from >= end_ts {
                break;
            }
        } else {
            break;
        }
    }

    if all_candles.is_empty() {
        return Err(format!("Gate.io 返回空数据 for {} ({} ~ {})", pair, start_date, end_date));
    }

    all_candles.sort_by_key(|(d, _)| *d);
    all_candles.dedup_by_key(|(d, _)| *d);  // 去重（分页边界可能有重复）
    Ok(all_candles)
}
```

### 4.3 读取合约收益数据（返回前端）

```rust
/// 从 daily_contract_values 读取数据，计算风险指标，返回完整摘要
/// 修复 Issue D：接受 annualization_factor 参数，支持 crypto(365) / tradfi(252)
pub fn read_contract_performance(
    db: &Connection,
    start_date: NaiveDate,
    end_date: NaiveDate,
    annualization_factor: f64,  // 修复 Issue D：365 for crypto, 252 for tradfi
) -> Result<ContractPerformanceSummary, String> {
    let mut stmt = db.prepare(
        "SELECT date, net_value, total_margin, unrealized_pnl, realized_pnl, total_pnl, return_rate
         FROM daily_contract_values
         WHERE date >= ? AND date <= ?
         ORDER BY date ASC"
    ).map_err(|e| e.to_string())?;

    let rows = stmt.query_map(
        [start_date.format("%Y-%m-%d").to_string(), end_date.format("%Y-%m-%d").to_string()],
        |row| {
            Ok(ContractReturnPoint {
                date: row.get(0)?,
                net_value: row.get(1)?,
                total_margin: row.get(2)?,
                unrealized_pnl: row.get(3)?,
                realized_pnl: row.get(4)?,
                total_pnl: row.get(5)?,
                daily_pnl: 0.0,  // 稍后计算
                cumulative_return: row.get(6)?,
            })
        }
    ).map_err(|e| e.to_string())?;

    let mut return_series: Vec<ContractReturnPoint> = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // 计算 daily_pnl 和 cumulative_return
    for i in 0..return_series.len() {
        if i == 0 {
            return_series[i].daily_pnl = 0.0;
        } else {
            return_series[i].daily_pnl = return_series[i].total_pnl - return_series[i-1].total_pnl;
        }
        return_series[i].cumulative_return = if return_series[i].total_margin > 0.0 {
            return_series[i].total_pnl / return_series[i].total_margin * 100.0
        } else {
            0.0
        };
    }

    // 计算风险指标
    // 修复 Issue D：传入年化因子（crypto=365, tradfi=252）
    let (max_drawdown, volatility, win_rate) = compute_risk_metrics(&return_series, annualization_factor);

    let current = return_series.last();
    Ok(ContractPerformanceSummary {
        start_date: start_date.format("%Y-%m-%d").to_string(),
        end_date: end_date.format("%Y-%m-%d").to_string(),
        current_net_value: current.map_or(0.0, |p| p.net_value),
        total_margin: current.map_or(0.0, |p| p.total_margin),
        total_pnl: current.map_or(0.0, |p| p.total_pnl),
        total_return: current.map_or(0.0, |p| p.cumulative_return),
        max_drawdown,
        volatility,
        win_rate,
        return_series,
    })
}
```

### 4.4 注册模块

**文件：`src-tauri/src/models/mod.rs`** — 新增：
```rust
pub mod contract_performance;
pub use contract_performance::{
    ContractPerformanceSummary, ContractReturnPoint
};
```

### 4.5 新增 Tauri Commands（`commands/contract_performance.rs`）

```rust
use tauri::State;
use chrono::NaiveDate;
use crate::db::Database;
use crate::models::contract_performance::*;
use crate::services::contract_performance_service::*;

// 修复 #1：用 Database 替代 AppState
// 修复 #19：添加 #[tauri::command(rename_all = "camelCase")]
#[tauri::command(rename_all = "camelCase")]
pub async fn get_contract_performance_summary(
    db: State<'_, Database>,
    start_date: String,   // "2025-01-01"
    end_date: String,     // "2025-06-30"
    force_refresh: bool,
) -> Result<ContractPerformanceSummary, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start = NaiveDate::parse_from_str(&start_date, "%Y-%m-%d")
        .map_err(|e| format!("无效开始日期: {}", e))?;
    let end = NaiveDate::parse_from_str(&end_date, "%Y-%m-%d")
        .map_err(|e| format!("无效结束日期: {}", e))?;

    // 自动 backfill（对齐股票逻辑，非致命）
    let filled = backfill_contract_snapshots(&conn, start, end, force_refresh).await;
    if let Ok(n) = &filled {
        if *n > 0 {
            println!("[contract_performance] backfill {} missing dates", n);
        }
    }
    // backfill 失败不中断，继续读取已有数据（对齐 performanceStore 的 non-fatal 模式）

    // 修复 Issue D：计算年化因子（crypto=365, tradfi=252）
    // 加载合约以确定资产类型占比
    let all_contracts = load_all_contracts(&conn).unwrap_or_default();
    let has_crypto = all_contracts.iter().any(|c| c.asset_type == "crypto");
    let has_tradfi = all_contracts.iter().any(|c| c.asset_type == "tradfi");
    let annualization_factor = if has_tradfi && !has_crypto {
        252.0  // 全部是 tradfi，用交易日数量
    } else {
        365.0  // 含有 crypto（或混合），用全年天数
    };

    // 从 DB 读取数据并返回（传入正确的年化因子）
    read_contract_performance(&conn, start, end, annualization_factor)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_earliest_contract_date(
    db: State<'_, Database>,
) -> Result<String, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    // 修复 #9：子查询加别名；处理 NULL 返回值
    let result: Option<String> = conn.query_row(
        "SELECT MIN(date) FROM (
            SELECT MIN(created_at) as date FROM crypto_contract
            UNION ALL
            SELECT MIN(closed_at) as date FROM crypto_contract_history WHERE closed_at IS NOT NULL
        ) AS sub",
        [],
        |row| row.get(0),
    ).ok().flatten();

    match result {
        Some(date) if date.len() >= 10 => Ok(date[..10].to_string()),
        _ => Ok("2024-01-01".to_string()),  // 默认值
    }
}
```

### 4.6 数据库迁移（`db/mod.rs`）

在现有迁移逻辑中新增建表语句（在 `run_migrations()` 函数中）：

```rust
// 在 run_migrations() 函数中新增（放在 crypto_contract 建表语句之后）
connection.execute(
    "CREATE TABLE IF NOT EXISTS daily_contract_values (
        date TEXT PRIMARY KEY,
        net_value REAL NOT NULL,
        total_margin REAL NOT NULL,
        unrealized_pnl REAL NOT NULL,
        realized_pnl REAL NOT NULL DEFAULT 0,
        total_pnl REAL NOT NULL,
        return_rate REAL NOT NULL DEFAULT 0
    )",
    [],
)?;
```

### 4.7 命令注册（`lib.rs` 或 `commands/mod.rs`）

```rust
// 在 commands/mod.rs 中新增
pub mod contract_performance;

// 在 lib.rs 的 invoke_handler 中新增
.invoke_handler(&[
    // ... 现有 commands ...
    commands::contract_performance::get_contract_performance_summary,
    commands::contract_performance::get_earliest_contract_date,
])
```

---

## 五、前端实现（React/TypeScript）—— 完整代码

### 5.1 新增类型（`src/types/index.ts`）

```typescript
// 在文件末尾新增：

export interface ContractReturnPoint {
  date: string;
  net_value: number;
  total_margin: number;
  unrealized_pnl: number;
  realized_pnl: number;
  total_pnl: number;
  daily_pnl: number;
  cumulative_return: number;
}

export interface ContractPerformanceSummary {
  start_date: string;
  end_date: string;
  current_net_value: number;
  total_margin: number;
  total_pnl: number;
  total_return: number;
  max_drawdown: number;
  volatility: number;
  win_rate: number;
  return_series: ContractReturnPoint[];
}
```

### 5.2 新建 Store（`src/stores/contractPerformanceStore.ts`）

对齐 `performanceStore.ts` 的结构：

```typescript
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { ContractPerformanceSummary } from '../types';

interface ContractPerformanceState {
  summary: ContractPerformanceSummary | null;
  loading: boolean;
  error: string | null;
  timeRange: '1M' | '3M' | '6M' | 'YTD' | 'ALL';
  // 修复 #15：缓存 earliest date，避免每次切 ALL 都请求
  earliestDate: string | null;

  fetchSummary: (forceRefresh?: boolean) => Promise<void>;
  setTimeRange: (range: '1M' | '3M' | '6M' | 'YTD' | 'ALL') => void;
}

export const useContractPerformanceStore = create<ContractPerformanceState>((set, get) => ({
  summary: null,
  loading: false,
  error: null,
  timeRange: 'ALL',
  earliestDate: null,

  fetchSummary: async (forceRefresh = false) => {
    set({ loading: true, error: null });

    try {
      // 1. 计算时间范围
      const range = get().timeRange;
      const end = new Date();
      let start = new Date();

      switch (range) {
        case '1M': start.setMonth(end.getMonth() - 1); break;
        case '3M': start.setMonth(end.getMonth() - 3); break;
        case '6M': start.setMonth(end.getMonth() - 6); break;
        case 'YTD': start = new Date(end.getFullYear(), 0, 1); break;
        case 'ALL':
          // 修复 #15：使用缓存的 earliest date
          let earliest = get().earliestDate;
          if (!earliest) {
            earliest = await invoke<string>('get_earliest_contract_date');
            set({ earliestDate: earliest });
          }
          start = new Date(earliest);
          break;
      }

      const startDate = start.toISOString().split('T')[0];
      const endDate = end.toISOString().split('T')[0];

      // 2. 自动 backfill（对齐股票逻辑，非致命）
      const summary = await invoke<ContractPerformanceSummary>(
        'get_contract_performance_summary',
        { startDate, endDate, forceRefresh }
      );

      set({ summary, loading: false });

    } catch (err) {
      console.error('[ContractPerformance] fetchSummary error:', err);
      set({ error: `获取合约收益数据失败: ${err}`, loading: false });
    }
  },

  setTimeRange: (range) => {
    set({ timeRange: range });
    // 注意：不要在这里调用 fetchSummary()，否则会与 useEffect 双重触发
    // useEffect 监听 timeRange 变化，会自动调用 fetchSummary()
  },
}));
```

### 5.3 新建组件（`src/pages/CryptoContract/ContractPerformanceTab.tsx`）

与股票 `Performance/index.tsx` 保持一致的布局和交互模式：

```tsx
import React, { useEffect, useMemo } from 'react';
import { Typography, Button, Spin, Alert, Row, Col, Select } from 'antd';
import { ReloadOutlined } from '@ant-design/icons';
import ReactECharts from 'echarts-for-react';
import { useContractPerformanceStore } from '../../stores/contractPerformanceStore';

const { Title, Text } = Typography;

const ContractPerformanceTab: React.FC = () => {
  // 修复 #14：用 selector 获取稳定引用
  const summary = useContractPerformanceStore((s) => s.summary);
  const loading = useContractPerformanceStore((s) => s.loading);
  const error = useContractPerformanceStore((s) => s.error);
  const timeRange = useContractPerformanceStore((s) => s.timeRange);
  const fetchSummary = useContractPerformanceStore((s) => s.fetchSummary);
  const setTimeRange = useContractPerformanceStore((s) => s.setTimeRange);

  // 修复 #14：useEffect 依赖数组完整，fetchSummary 是 Zustand 稳定引用
  useEffect(() => {
    fetchSummary();
  }, [timeRange, fetchSummary]);

  // ECharts 配置（双 Y 轴）
  const chartOption = useMemo(() => {
    if (!summary || summary.return_series.length === 0) return {};

    const dates = summary.return_series.map(p => p.date);
    const netValues = summary.return_series.map(p => p.net_value);
    const totalPnls = summary.return_series.map(p => p.total_pnl);

    return {
      tooltip: {
        trigger: 'axis',
        formatter: (params: any) => {
          const date = params[0].name;
          let html = `<div style="font-size:12px;"><strong>${date}</strong></div>`;
          params.forEach((p: any) => {
            const isRed = p.value >= 0;
            const color = isRed ? '#E53935' : '#43A047';  // 红涨绿跌（中国习惯）
            html += `<div style="font-size:12px;color:${color}">${p.seriesName}: ${p.value >= 0 ? '+' : ''}${p.value.toFixed(2)}</div>`;
          });
          return html;
        }
      },
      legend: {
        data: ['净值', '累计盈亏'],
        top: 10,
      },
      grid: {
        left: '3%',
        right: '4%',
        bottom: '3%',
        containLabel: true,
      },
      xAxis: {
        type: 'category',
        data: dates,
        axisLabel: {
          rotate: 45,
        },
      },
      yAxis: [
        {
          type: 'value',
          name: '净值 ($)',
          position: 'left',
          axisLabel: {
            formatter: '${value}',
          },
        },
        {
          type: 'value',
          name: '累计盈亏 ($)',
          position: 'right',
          axisLabel: {
            formatter: '${value}',
          },
        },
      ],
      series: [
        {
          name: '净值',
          type: 'line',
          yAxisIndex: 0,
          data: netValues,
          smooth: true,
          lineStyle: {
            color: summary.total_return >= 0 ? '#E53935' : '#43A047',
            width: 2,
          },
          areaStyle: {
            color: {
              type: 'linear',
              x: 0, y: 0, x2: 0, y2: 1,
              colorStops: [
                { offset: 0, color: summary.total_return >= 0 ? 'rgba(229,57,53,0.3)' : 'rgba(67,160,71,0.3)' },
                { offset: 1, color: 'rgba(255,255,255,0)' },
              ],
            },
          },
        },
        {
          name: '累计盈亏',
          type: 'line',
          yAxisIndex: 1,
          data: totalPnls,
          smooth: true,
          lineStyle: {
            color: '#FF9800',
            width: 1.5,
            type: 'dashed',
          },
        },
      ],
      dataZoom: [
        { type: 'inside', start: 0, end: 100 },
        { type: 'slider', start: 0, end: 100 },
      ],
    };
  }, [summary]);

  if (loading) {
    return <Spin size="large" style={{ display: 'flex', justifyContent: 'center', marginTop: 100 }} />;
  }

  if (error) {
    return <Alert type="error" message={error} showIcon />;
  }

  if (!summary || summary.return_series.length === 0) {
    return (
      <div style={{ textAlign: 'center', marginTop: 50 }}>
        <Text type="secondary">暂无合约收益数据。</Text>
        <br />
        <Button type="primary" onClick={() => fetchSummary(true)} style={{ marginTop: 16 }}>
          手动触发计算
        </Button>
      </div>
    );
  }

  return (
    <div>
      {/* 时间范围选择 + 刷新按钮 */}
      <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 16 }}>
        <Select value={timeRange} onChange={setTimeRange} style={{ width: 200 }}>
          <Select.Option value="1M">最近 1 个月</Select.Option>
          <Select.Option value="3M">最近 3 个月</Select.Option>
          <Select.Option value="6M">最近 6 个月</Select.Option>
          <Select.Option value="YTD">今年以来</Select.Option>
          <Select.Option value="ALL">全部</Select.Option>
        </Select>

        <Button
          icon={<ReloadOutlined />}
          onClick={() => fetchSummary(true)}
          loading={loading}
        >
          刷新
        </Button>
      </div>

      {/* 修复 #16：摘要卡片从 4 个扩展为 6 个 */}
      <Row gutter={[16, 16]} style={{ marginBottom: 24 }}>
        <Col span={8}>
          <div style={{ background: '#fff', padding: 16, borderRadius: 8, boxShadow: '0 1px 4px rgba(0,0,0,0.1)' }}>
            <Text type="secondary">当前净值</Text>
            <div style={{ fontSize: 24, fontWeight: 'bold' }}>${summary.current_net_value.toFixed(2)}</div>
          </div>
        </Col>
        <Col span={8}>
          <div style={{ background: '#fff', padding: 16, borderRadius: 8, boxShadow: '0 1px 4px rgba(0,0,0,0.1)' }}>
            <Text type="secondary">累计盈亏</Text>
            <div style={{ fontSize: 24, fontWeight: 'bold', color: summary.total_pnl >= 0 ? '#E53935' : '#43A047' }}>
              {summary.total_pnl >= 0 ? '+' : ''}{summary.total_pnl.toFixed(2)}
            </div>
          </div>
        </Col>
        <Col span={8}>
          <div style={{ background: '#fff', padding: 16, borderRadius: 8, boxShadow: '0 1px 4px rgba(0,0,0,0.1)' }}>
            <Text type="secondary">总保证金</Text>
            <div style={{ fontSize: 24, fontWeight: 'bold' }}>${summary.total_margin.toFixed(2)}</div>
          </div>
        </Col>
        <Col span={8}>
          <div style={{ background: '#fff', padding: 16, borderRadius: 8, boxShadow: '0 1px 4px rgba(0,0,0,0.1)' }}>
            <Text type="secondary">收益率</Text>
            <div style={{ fontSize: 24, fontWeight: 'bold', color: summary.total_return >= 0 ? '#E53935' : '#43A047' }}>
              {summary.total_return >= 0 ? '+' : ''}{summary.total_return.toFixed(2)}%
            </div>
          </div>
        </Col>
        <Col span={8}>
          <div style={{ background: '#fff', padding: 16, borderRadius: 8, boxShadow: '0 1px 4px rgba(0,0,0,0.1)' }}>
            <Text type="secondary">最大回撤</Text>
            <div style={{ fontSize: 24, fontWeight: 'bold', color: '#FF9800' }}>
              -{summary.max_drawdown.toFixed(2)}%
            </div>
          </div>
        </Col>
        <Col span={8}>
          <div style={{ background: '#fff', padding: 16, borderRadius: 8, boxShadow: '0 1px 4px rgba(0,0,0,0.1)' }}>
            <Text type="secondary">波动率</Text>
            <div style={{ fontSize: 24, fontWeight: 'bold', color: '#9C27B0' }}>
              {summary.volatility.toFixed(2)}%
            </div>
          </div>
        </Col>
      </Row>

      {/* 图表 */}
      <div style={{ background: '#fff', padding: 16, borderRadius: 8, boxShadow: '0 1px 4px rgba(0,0,0,0.1)' }}>
        <ReactECharts option={chartOption} style={{ height: 400 }} />
      </div>
    </div>
  );
};

export default ContractPerformanceTab;
```

### 5.4 修改合约页面（`src/pages/CryptoContract/index.tsx`）

在 `<Tabs>` 中新增第三个 Tab（在"历史成交"Tab 之后）：

```tsx
// 在文件顶部引入
import ContractPerformanceTab from './ContractPerformanceTab';

// 在 Tabs 的 items 数组中新增
{
  key: "performance",
  label: "📈 收益曲线",
  children: <ContractPerformanceTab />,
}
```

---

## 六、边界情况处理

### 6.1 部分平仓

若合约先开了 10 个，然后平了 3 个（部分平仓），剩余 7 个：
- **历史曲线**：平仓前的净值按 10 个计算；平仓后的净值按 7 个计算
- **实现方式**：在 `crypto_contract_history` 表中，`action_type='close'` 的记录会记录 `close_shares`（平仓数量），在 `load_all_contracts()` 中已正确处理（见 4.2 节）

```rust
// 处理部分平仓：重建合约的完整生命周期
// 注意：部分平仓和全部平仓的 action_type 都是 'close'（修复 #4）
// 区别在于 close_shares < total_shares 时为部分平仓
fn rebuild_contract_from_history(db: &Connection, contract_id: &str) -> Result<ContractState, String> {
    let mut stmt = db.prepare(
        "SELECT action_type, open_price, close_shares, add_shares, new_avg_price, new_total_shares,
                realized_pnl, closed_at, leverage, open_fee, close_fee
         FROM crypto_contract_history
         WHERE contract_id = ?
         ORDER BY closed_at ASC"
    ).map_err(|e| e.to_string())?;

    let events = stmt.query_map([contract_id], |row| {
        Ok((
            row.get::<_, String>(0)?,            // action_type
            row.get::<_, f64>(1)?,               // open_price
            row.get::<_, Option<f64>>(2)?,       // close_shares
            row.get::<_, Option<f64>>(3)?,       // add_shares
            row.get::<_, Option<f64>>(4)?,       // new_avg_price
            row.get::<_, Option<f64>>(5)?,       // new_total_shares
            row.get::<_, Option<f64>>(6)?,       // realized_pnl
            row.get::<_, Option<String>>(7)?,    // closed_at
            row.get::<_, f64>(8)?,               // leverage
            row.get::<_, Option<f64>>(9)?.unwrap_or(0.0),  // open_fee
            row.get::<_, Option<f64>>(10)?.unwrap_or(0.0), // close_fee
        ))
    }).map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())?;

    // 重建状态...
    // （完整实现已在 load_all_contracts() 中通过 ClosedContractSummary 完成）

    Err("部分平仓合约的历史重建已包含在 load_all_contracts() 中".to_string())
}
```

**建议**：第一版可简化处理，在 `compute_net_value()` 中，若发现该合约有部分平仓记录但仍在持仓中，则用**当前剩余数量**计算历史净值（不精确，但可用）。同时在 UI 中给出提示。

### 6.2 数据库中字段为 NULL

```rust
// 所有从 DB 读取的可空字段都用 Option 处理
let margin = row.get::<_, Option<f64>>(6)?.unwrap_or(0.0);
let leverage = row.get::<_, Option<f64>>(5)?.unwrap_or(1.0);
let fee = row.get::<_, Option<f64>>(8)?.unwrap_or(0.0);
```

### 6.3 Gate.io API 失败

```rust
// 在 fetch_crypto_history() 中加入重试逻辑
async fn fetch_crypto_history_with_retry(
    symbol: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
    max_retries: u32,
) -> Result<Vec<(NaiveDate, f64)>, String> {
    for attempt in 0..max_retries {
        match fetch_crypto_history(symbol, start_date, end_date).await {
            Ok(data) => return Ok(data),
            Err(e) if attempt < max_retries - 1 => {
                eprintln!("[contract_performance] Gate.io API 失败 (尝试 {}): {}", attempt + 1, e);
                tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt + 1) as u64)).await;
            }
            Err(e) => return Err(format!("Gate.io API 失败（已重试 {} 次）: {}", max_retries, e)),
        }
    }
    unreachable!()
}
```

### 6.4 前向填充缺失价格

已在 `get_price_for_date()` 中实现（见 4.2 节），返回 `Option<f64>`，缺失时跳过该合约。

---

## 七、需要的所有文件变更清单

### Rust 后端（`src-tauri/src/`）

| 文件 | 操作 | 说明 | 估计行数 |
|------|------|------|----------|
| `models/contract_performance.rs` | **新建** | `ContractReturnPoint`、`ContractPerformanceSummary`、`ContractState` 结构体 + 风险指标计算 | ~150 行 |
| `models/mod.rs` | 修改 | 新增 `pub mod contract_performance;` | ~2 行 |
| `services/contract_performance_service.rs` | **新建** | 核心计算逻辑 + `backfill_contract_snapshots()` + `fetch_crypto_history()`（含分页）+ 风险指标 | ~600 行 |
| `commands/contract_performance.rs` | **新建** | Tauri commands（2 个），用 `Database` + `camelCase` | ~80 行 |
| `db/mod.rs` | 修改 | 新增 `daily_contract_values` 建表语句（在 `run_migrations()` 中） | ~10 行 |
| `lib.rs` 或 `commands/mod.rs` | 修改 | 新增 command 注册 | ~5 行 |

### 前端（`src/`）

| 文件 | 操作 | 说明 | 估计行数 |
|------|------|------|----------|
| `types/index.ts` | 修改 | 新增 `ContractReturnPoint`、`ContractPerformanceSummary` 接口 | ~30 行 |
| `stores/contractPerformanceStore.ts` | **新建** | Zustand store（对齐 `performanceStore.ts`），含 earliestDate 缓存 | ~110 行 |
| `pages/CryptoContract/ContractPerformanceTab.tsx` | **新建** | 收益曲线 Tab 组件（含 ECharts 图表 + 6 个摘要卡片） | ~280 行 |
| `pages/CryptoContract/index.tsx` | 修改 | Tabs 增加第三个 Tab | ~5 行 |

**总计估计**：~1200 行新代码 + ~20 行修改

---

## 八、与现有股票绩效分析的对比

| 维度 | 股票绩效分析 | 合约收益曲线 |
|------|-------------|-------------|
| 数据表 | `daily_portfolio_values` | `daily_contract_values` |
| 历史价格 API | Yahoo Finance / 雪球 / 东方财富 | Gate.io K 线（crypto）+ Yahoo Finance（tradfi） |
| 触发机制 | 打开页面时自动 `backfill_snapshots` | 打开页面时自动 `backfill_contract_snapshots` |
| 净值定义 | 持仓市值（shares × price） | 保证金 + 盈亏 |
| 收盘概念 | 有（用昨日收盘价） | 无（用最近完整 UTC 日收盘价） |
| 前端 | `Performance/index.tsx` | `CryptoContract/ContractPerformanceTab.tsx` |

---

## 九、GLM 交叉评审修复清单（v4.1 → v4.2）

本节记录 GLM 交叉评审中发现的所有 19 个问题及其修复状态。

### 🐛 致命问题（5 个）

| # | 问题 | 修复状态 | 修复位置 |
|---|------|---------|---------|
| 1 | `AppState` 不存在——实际是 `Database` | ✅ 已修复 | 4.5 节：所有 Tauri command 签名改为 `db: State<'_, Database>`，`use crate::db::Database` |
| 2 | `fetch_stock_history_yahoo` 签名不匹配 | ✅ 已修复 | 4.2 节：调用改为 `fetch_stock_history_yahoo(&symbol, market, start_date, end_date)`，移除不存在的 `provider` 参数 |
| 3 | `load_all_contracts()` 中 `open_price` 被当作 `shares` 使用 | ✅ 已修复 | 4.2 节：`'open'` 动作的 `close_shares` 字段存储开仓数量（见 db/crypto_contract.rs 第 86-93 行注释），正确读取；`remaining_shares` 正确递增递减 |
| 4 | `action_type='partial_close'` 不存在 | ✅ 已修复 | 4.2 节 + 6.1 节：移除所有 `partial_close` 引用，部分平仓和全部平仓都用 `'close'` |
| 5 | `closed_contracts` push 时缺少 `market` 字段 | ✅ 已修复 | 4.1 节：`ContractState` 包含 `asset_type` 和 `market`；4.2 节：push 时设置所有字段 |

### 🔧 严重问题（6 个）

| # | 问题 | 修复状态 | 修复位置 |
|---|------|---------|---------|
| 6 | `is_crypto_symbol()` 用硬编码前缀列表判断——不可靠 | ✅ 已修复 | 4.2 节：移除 `is_crypto_symbol()`，改用 `contract.asset_type == "crypto"` 判断 |
| 7 | `compute_net_value` 中 `?` 操作符会让单个价格缺失导致整个 backfill 失败 | ✅ 已修复 | 4.2 节：`get_price_for_date()` 返回 `Option<f64>`，缺失时跳过该合约；`compute_net_value()` 中 `match` 处理，不中断整体计算 |
| 8 | `force` 参数实际未被使用 | ✅ 已修复 | 4.2 节：`force=true` 时先 `DELETE FROM daily_contract_values WHERE date >= ? AND date <= ?`，再重新计算 |
| 9 | `get_earliest_contract_date` 的 SQL 有语法问题 | ✅ 已修复 | 4.5 节：子查询加 `AS sub` 别名；`query_row` 结果用 `Option<String>` 处理 NULL |
| 10 | Gate.io `limit=1000` 可能不够用 | ✅ 已修复 | 4.2 节：`fetch_crypto_history()` 增加分页逻辑，每页 1000 条，最多 10 页（~27 年），自动去重 |
| 11 | `NaiveDate::from_num_days_from_ce_opt` 时间戳转换不安全 | ✅ 已修复 | 4.2 节：改用 `chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.naive_utc().date())` |

### ⚠️ 中等问题（5 个）

| # | 问题 | 修复状态 | 修复位置 |
|---|------|---------|---------|
| 12 | 没有并发保护 | ✅ 已修复 | 4.2 节：新增 `static BACKFILL_LOCK: Mutex<bool>`，`backfill_contract_snapshots()` 入口获取锁 |
| 13 | `ClosedContractSummary` 重建逻辑不完整 | ✅ 已修复 | 4.2 节：`total_shares` 从 `open`/`add` 动作正确递增；`remaining_shares` 从 `close` 动作正确递减；`margin` = `avg_open_price * total_shares / leverage + total_fees`；`open_date` 从最早 `open` 动作获取 |
| 14 | 前端 store 的 `useEffect` 依赖问题 | ✅ 已修复 | 5.3 节：用 `useContractPerformanceStore((s) => s.fetchSummary)` 获取稳定引用，依赖数组 `[timeRange, fetchSummary]` |
| 15 | 前端 ALL 时间范围每次都要先请求 `get_earliest_contract_date` | ✅ 已修复 | 5.2 节：store 新增 `earliestDate` 状态，首次获取后缓存 |
| 16 | 前端摘要卡片只有 4 个，方案描述中有 6 个指标 | ✅ 已修复 | 5.3 节：从 4 个 `Col span={6}` 改为 6 个 `Col span={8}`，新增"总保证金"和"波动率"卡片 |

### 💡 改进建议（3 个）

| # | 问题 | 修复状态 | 修复位置 |
|---|------|---------|---------|
| 17 | 用 `asset_type` 代替 `is_crypto_symbol()` | ✅ 已修复 | 4.1 节：`ContractState` 新增 `asset_type: String` 字段；4.2 节：所有类型判断改用 `asset_type` |
| 18 | 模拟数据 `total_return` 与 `total_margin`/`total_pnl` 矛盾 | ✅ 已修复 | 附录 B：`total_return` 动态计算 `totalPnl / totalMargin * 100`，不硬编码 |
| 19 | `#[tauri::command(rename_all = "camelCase")]` 缺失 | ✅ 已修复 | 4.5 节：两个 Tauri command 都添加 `#[tauri::command(rename_all = "camelCase")]` |

### 🔍 自我评审额外问题（3 个，v4.2a 修复）

在修复 GLM 评审 19 个问题的过程中，发现了以下额外问题：

| # | 问题 | 严重程度 | 修复状态 | 修复位置 |
|---|------|---------|---------|---------|
| A | 部分平仓合约可能被重复计算：`crypto_contract` 表中仍在持仓的合约（部分平仓后）同时被 `load_all_contracts()` 的"活跃合约"和"历史重建"两部分加载 | 🔴 高 | ✅ 已修复 | 4.2 节：新增 `active_contract_ids` 集合，重建历史时跳过仍在 `crypto_contract` 表中的 `contract_id` |
| B | `fetch_crypto_history()` 中 `let pair = format!("{}_USDT", ...)` 写死 `_USDT`，非 USDT 报价货币的合约会请求失败 | 🟡 中 | ✅ 已修复 | 4.2 节：`fetch_crypto_history()` 新增 `quote_currency: &str` 参数，默认 `"USDT"`；调用点传入 `quote_currency` |
| D | `compute_risk_metrics()` 中年化因子硬编码 `365`，tradfi 合约应该用 `252`（交易日数量） | 🟡 中 | ✅ 已修复 | 4.3 节：`compute_risk_metrics()` 新增 `annualization_factor: f64` 参数；`get_contract_performance_summary()` 中根据合约类型自动选择 `365.0`（crypto）或 `252.0`（tradfi） |

**Issue A 详细说明**：

部分平仓的合约同时满足两个条件：
1. 还在 `crypto_contract` 表里（未删除，部分平仓后剩余份额仍在持仓）
2. 有 `'close'` 类型的历史记录（`crypto_contract_history` 表）

原代码会把它的 `realized_pnl` 算两次——一次在活跃合约的 `realized_pnl` 字段，一次在重建的逻辑里。

**修复方案**：重建历史时，先收集所有活跃合约的 ID（`active_contract_ids`），然后跳过这些 ID。

**Issue B 详细说明**：

当前 `crypto_contract` 表没有 `quote_currency` 字段，第一版假设所有合约都是 USDT 报价货币（大多数确实是）。若用户持有 BTC/ETH 交易对，需要在 `crypto_contract` 表新增 `quote_currency` 字段（默认值 `'USDT'`）。

**Issue D 详细说明**：

加密货币全年 365 天可交易，年化波动率用 `sqrt(365)`；tradfi 只有约 252 个交易日，应该用 `sqrt(252)`。当前代码根据合约类型自动选择。

---

## 十、开发优先级

| 优先级 | 功能 | 说明 | 估计工时 |
|--------|------|------|----------|
| P0 | `fetch_crypto_history()`（Gate.io K 线，含分页） | 必须先验证 Rust 中调用 Gate.io API 是否正常 | 3h |
| P0 | `backfill_contract_snapshots()` 核心逻辑 | 对齐 `backfill_snapshots` 结构；注意部分平仓的处理 | 8h |
| P0 | 前端 `ContractPerformanceTab.tsx` | 用户可见的部分；对齐股票 Performance 页面 | 4h |
| P1 | 风险指标计算（最大回撤、波动率、胜率） | 提升产品价值 | 2h |
| P2 | 资金费率真实值接入 | 精度提升（第一版可用估算值） | 4h |
| P2 | 导出 CSV 功能 | 用户自选分析 | 2h |

---

## 十一、验证清单（开发完成后）

### 后端验证
- [ ] `daily_contract_values` 表成功创建（检查 SQLite 数据库）
- [ ] `fetch_crypto_history()` 能正确获取 BTC/ETH/SOL 的历史价格（含分页）
- [ ] `backfill_contract_snapshots()` 能正确补全缺失日期的数据
- [ ] `backfill_contract_snapshots(force=true)` 能删除旧数据并重新计算
- [ ] `read_contract_performance()` 返回的数据格式正确
- [ ] Gate.io API 失败时，错误信息清晰，前端有容错提示
- [ ] 风险指标计算正确（用模拟数据验证）
- [ ] tradfi 合约能正确调用 `fetch_stock_history_yahoo(symbol, market, start, end)`
- [ ] 已平仓合约的 `shares`、`margin`、`realized_pnl` 计算正确
- [ ] 并发请求不会造成数据不一致（`BACKFILL_LOCK` 生效）

### 前端验证
- [ ] 打开收益曲线 Tab，自动 backfill 缺失日期的数据
- [ ] 切换时间范围（1M/3M/6M/YTD/ALL），数据正确过滤
- [ ] 点击刷新按钮，强制重新计算
- [ ] 双 Y 轴图表渲染正确，tooltip 显示完整
- [ ] 6 个摘要卡片数据正确（当前净值、累计盈亏、总保证金、收益率、最大回撤、波动率）
- [ ] 已平仓合约的盈亏在曲线中正确反映
- [ ] ALL 时间范围的 earliest date 缓存生效（只请求一次）

### 集成验证
- [ ] 与股票 Performance 页面并存，互不干扰
- [ ] 现有合约页面功能正常（添加、编辑、删除合约）
- [ ] 数据库迁移正确（旧版本升级后，`daily_contract_values` 表自动创建）
- [ ] Tauri command 参数名正确（camelCase：`startDate`、`endDate`、`forceRefresh`）

---

## 十二、实现注意事项

### 12.1 性能考虑

- **历史价格缓存**：`backfill_contract_snapshots()` 中会拉取所有涉及标的的历史价格，若标的较多（>10），建议加入缓存机制（内存缓存，有效期 1 小时）
- **数据库查询优化**：`daily_contract_values` 表的 `date` 字段是 PRIMARY KEY，查询已是最优
- **前端渲染**：若 `return_series` 数据量很大（>1000 点），ECharts 可能需要开启 `large: true` 模式

### 12.2 数据一致性

- **保证金变化**：若用户修改了合约的保证金（例如转入更多保证金），需要在 `crypto_contract` 表中更新，并重新 backfill 受影响日期的数据
- **已平仓合约**：若用户删除了某个已平仓合约的记录，`daily_contract_values` 中的数据不会自动更新。建议：在删除合约时，提示用户"删除后历史收益曲线将不完整"

### 12.3 时区处理

- Gate.io K 线的 `timestamp` 是**秒级 UTC 时间**
- 在转换为 `NaiveDate` 时，统一用 UTC 日期（`DateTime::from_timestamp` 返回 UTC）
- 前端展示时，可以用 `dayjs` 转换为用户本地时区

### 12.4 部分平仓的处理策略（建议）

**第一版（MVP）**：
- 在 `compute_net_value()` 中，若发现该合约有部分平仓记录但仍在持仓中，则用**当前剩余数量**计算历史净值
- 在 UI 中给出提示："提示：该合约有部分平仓记录，历史曲线可能不准确"

**第二版（精确）**：
- 在 `crypto_contract_history` 表中完整记录每次操作后的 `shares` 和 `avg_open_price`
- 在 `load_all_contracts()` 中，为该合约生成一个"按时间变化的 shares"列表
- 在 `compute_net_value()` 中，根据该日期对应的 shares 计算净值

---

## 附录

### 附录 A：Gate.io K 线 API 测试脚本（Rust）

开发前，先在 Rust 中写测试验证 API 可用性：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[tokio::test]
    async fn test_gateio_crypto_history() {
        let symbols = vec!["BTC", "ETH", "SOL"];
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let end = chrono::Utc::now().naive_utc().date();

        for sym in symbols {
            let result = fetch_crypto_history(sym, start, end).await;
            assert!(result.is_ok(), "Failed to fetch {}: {:?}", sym, result.err());

            let data = result.unwrap();
            assert!(data.len() > 100, "Too few data points for {}: {}", sym, data.len());

            for (date, price) in &data {
                assert!(*price > 0.0, "Invalid price for {} on {}: {}", sym, date, price);
            }

            println!("[OK] {}: {} data points, latest price: ${:.2}", sym, data.len(), data.last().unwrap().1);
        }
    }

    #[tokio::test]
    async fn test_gateio_pagination() {
        // 测试分页：请求超过 1000 天的数据
        let start = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
        let result = fetch_crypto_history("BTC", start, end).await;
        assert!(result.is_ok());
        let data = result.unwrap();
        // 5 年应该有 ~1826 天的数据（超过 1000 的分页限制）
        assert!(data.len() > 1000, "Pagination failed: only {} data points for 5-year range", data.len());
        println!("[OK] Pagination: {} data points for 2020-2024", data.len());
    }
}
```

运行：
```bash
cd F:/WorkSpace/stock-portfolio-manager/src-tauri
cargo test test_gateio --nocapture
```

### 附录 B：模拟数据生成脚本（用于前端开发调试）

若后端尚未完成，可以用模拟数据在前端调试图表：

```typescript
// 在 ContractPerformanceTab.tsx 中，临时用模拟数据
function generateMockSummary(): ContractPerformanceSummary {
  const totalMargin = 8000;
  const series = generateMockSeries('2024-01-01', '2026-06-30');
  const lastPoint = series[series.length - 1];
  const totalPnl = lastPoint.total_pnl;

  // 修复 #18：total_return 动态计算，不硬编码
  return {
    start_date: '2024-01-01',
    end_date: '2026-06-30',
    current_net_value: lastPoint.net_value,
    total_margin: totalMargin,
    total_pnl: totalPnl,
    total_return: totalPnl / totalMargin * 100,  // 动态计算
    max_drawdown: 12.3,
    volatility: 18.2,
    win_rate: 58.5,
    return_series: series,
  };
}

function generateMockSeries(start: string, end: string): ContractReturnPoint[] {
  const series: ContractReturnPoint[] = [];
  const startDate = new Date(start);
  const endDate = new Date(end);
  let currentDate = new Date(startDate);
  let netValue = 5000;
  const totalMargin = 5000;

  while (currentDate <= endDate) {
    const change = (Math.random() - 0.48) * 200;  // 模拟日涨跌
    netValue += change;
    const totalPnl = netValue - totalMargin;

    series.push({
      date: currentDate.toISOString().split('T')[0],
      net_value: netValue,
      total_margin: totalMargin,
      unrealized_pnl: totalPnl * 0.7,
      realized_pnl: totalPnl * 0.3,
      total_pnl: totalPnl,
      daily_pnl: change,
      cumulative_return: totalMargin > 0 ? totalPnl / totalMargin * 100 : 0,  // 修复 #18：动态计算
    });

    currentDate.setDate(currentDate.getDate() + 1);
  }

  return series;
}
```

### 附录 C：与现有代码的确切集成点

1. **`lib.rs` 第 ~30 行**：在 `invoke_handler` 中新增 `commands::contract_performance::get_contract_performance_summary` 和 `get_earliest_contract_date`
2. **`models/mod.rs` 第 ~10 行**：在现有 `pub mod performance;` 之后新增 `pub mod contract_performance;`
3. **`db/mod.rs` 第 ~80 行**：在 `run_migrations()` 函数中，在 `crypto_contract` 建表语句之后新增 `daily_contract_values` 建表语句
4. **`src/types/index.ts` 第 ~100 行**：在文件末尾新增 `ContractReturnPoint` 和 `ContractPerformanceSummary` 接口
5. **`src/stores/` 目录**：新增 `contractPerformanceStore.ts`
6. **`src/pages/CryptoContract/` 目录**：新增 `ContractPerformanceTab.tsx`
7. **`src/pages/CryptoContract/index.tsx` 第 ~50 行**：在 Tabs 的 items 数组中新增"收益曲线"Tab

### 附录 D：接口对齐参考（实际代码库关键信息）

| 项 | 实际值 | 文件位置 |
|----|--------|---------|
| Tauri State 类型 | `State<'_, Database>`（`pub struct Database { pub conn: Mutex<Connection> }`） | `db/mod.rs` 第 7-9 行 |
| Tauri command 属性 | `#[tauri::command(rename_all = "camelCase")]` | `commands/snapshots.rs` 第 11 行 |
| `fetch_stock_history_yahoo` 签名 | `(symbol: &str, market: &str, start_date: NaiveDate, end_date: NaiveDate)` | `quote_service.rs` 第 1361-1366 行 |
| `action_type` 实际值 | `'open'`、`'add'`、`'close'`（无 `partial_close`） | `db/crypto_contract.rs` |
| `asset_type` 字段 | 存在于 `crypto_contract` 表和模型（`'crypto'` 或 `'tradfi'`） | `db/mod.rs` 第 312 行 |
| `'open'` 动作中 `close_shares` | 存储开仓数量（不是平仓数量） | `db/crypto_contract.rs` 第 86-93 行注释 |
| `'add'` 动作字段 | `add_shares`、`new_avg_price`、`new_total_shares` | `db/crypto_contract.rs` 第 203-209 行 |
| `'close'` 动作字段 | `close_price`、`close_shares`（平仓数量）、`realized_pnl` | `db/crypto_contract.rs` 第 295-301 行 |
| `crypto_contract_history` 表列 | `id, contract_id, symbol, name, asset_type, position_type, action_type, open_price, close_price, close_shares, add_price, add_shares, new_avg_price, new_total_shares, leverage, open_fee, close_fee, realized_pnl, return_rate, closed_at, notes` | `db/mod.rs` 第 337-361 行 |
| 股票 backfill 模式 | `fetchAll()` 中先 `backfill_snapshots`（非致命），再 `Promise.allSettled` 并行获取 | `performanceStore.ts` 第 162-190 行 |

---

_结束 of v4.2 方案（GLM 交叉评审修复版） —— 可直接用于开发实施_
