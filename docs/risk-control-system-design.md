# 风控系统设计 · Risk & Discipline Engine

> 项目：stock-portfolio-manager（Tauri 2 + Rust + React + SQLite，本地单机）
> 设计依据：`all_documents.zip` 六份参考材料（风和亚洲理念 / RiskControl / 投资镜子 PIP / 过程即奖励）
> 版本：v1.3 · 2026-11-07 · 状态：**P0–P4 全部已实施**（实施记录见 §16–§20；问题与优化清单见 `docs/risk-system-review.md`）

---

## 0. 结论先行

**建议做，但不能照搬参考系统。** 六份材料描述的是两套纯前端 Web 应用（React + Supabase + 云 RAG），而本项目是**本地 Tauri 桌面应用 + SQLite**，架构前提完全不同。真正值得移植的是 **「用代码固化交易纪律」的设计哲学**，而不是它的技术栈。

落地结论：

| 判断 | 内容 |
|---|---|
| **核心移植** | 三大风控铁律（止损 / 回撤熔断 / 集中度）、防 FOMO 冷静期、HWM 水位线、LIFO 复盘、时光机、辩证式 AI 反馈 |
| **必须改造** | HWM 改用 **TWR 单位净值**而非总净值（参考实现有出入金导致假回撤的缺陷）；规则引擎放 Rust 层而非前端；云 RAG 改为本地检索 |
| **明确不做** | Supabase 云同步、暗黑 Bloomberg 风、邮件推送、强制只显示比例、SHORT/COVER 交易类型 |
| **前置依赖** | 项目**缺少显式的出/入金事件**，会让 TWR/HWM 失真，需先补 `external_cash_flows`（见 §7.3） |
| **数据铁律** | 风控引擎**禁止用日快照判定持仓**，必须用实时 `holdings` + 行情（本次会话已两次踩坑，见 §5.5） |
| **实施节奏** | P0 规则引擎 → P1 防 FOMO 拦截 → P2 复盘 → P3 AI 顾问 → P4 数据质量，每阶段独立可用 |

现有项目已有约 **60% 的基础设施**（多币种折算、行情缓存、每日净值快照、价格提醒、决策质量复盘、AI 配置表），风控系统主要是**补规则层与编排层**，不是从零造轮子。

---

## 1. 设计依据：参考材料解读

### 1.1 两套参考系统

| 系统 | 出处文件 | 灵魂 | 关键机制 |
|---|---|---|---|
| **RiskControl** 全栈风控与复盘 | `FullStack_Risk_System.ini`、`RiskControl_Prompt.ini` | 「只有偏执狂才能生存」——假设人性脆弱，用代码固化理性 | 三大铁律、7 日冷静期、HWM 熔断、LIFO 配对、时光机、压力测试、混合行情、HWM 云端取 max |
| **投资镜子 PIP** / 过程即奖励的持仓 | `Investment_Mirror_Intro.md`、`Manus_Build_Guide.md`、`AI_Advisor_Prompt.md`、`Process_As_Reward.md` | 「过程即奖励，找到那把属于自己的剑」——记录、分析、反思 | 辩证式反馈、挑战假设、有据可查、性格画像/认知偏差、投资笔记+持仓快照、每日 9:15 交易分析、隐私优先 |

### 1.2 「风和」理念的三层内核

```
第一层  保护本金（Defend）     →  止损 / 回撤熔断 / 仓位上限：先活下来
第二层  约束行为（Discipline） →  冷静期 / 非计划交易标记 / 强制复盘：对抗 FOMO 与情绪
第三层  认知进化（Reflection） →  记录「为什么买」/ 辩证式质疑 / 决策质量评分：过程即奖励
```

**关键洞察**：参考系统里最值钱的不是那些数字阈值，而是**「把纪律做成不可绕过的流程」**——买入前必须过观察期，30 天后必须回头看当初的理由，AI 的职责不是给建议而是挑战你。阈值只是载体。

### 1.3 理念 → 能力映射

| 理念 | 具体能力 | 本项目落点 |
|---|---|---|
| 保护本金 | 止损红线 −20% | 新增规则 `STOP_LOSS` |
| 保护本金 | 最大回撤 −5% 熔断 | 新增 `MAX_DRAWDOWN` + `portfolio_hwm` |
| 保护本金 | 个股集中度 15% | 新增 `CONCENTRATION` |
| 约束行为 | 观察列表 + 7 日冷静期 | 新增 `watchlist` + 交易前置拦截 |
| 约束行为 | 非计划交易警告 | 新增 `UNPLANNED_TRADE` |
| 约束行为 | 违规留痕 | 新增 `trade_discipline` |
| 认知进化 | 投资笔记 + 持仓快照 | 新增 `investment_memos` |
| 认知进化 | 30 天强制复盘 / 时光机 | 新增 `trade_reviews.due_at` |
| 认知进化 | LIFO 完整交易配对 | 新增 `review_service` 扩展 |
| 认知进化 | 决策质量 | 复用 `quarterly_holding_snapshots.decision_quality`，下沉到交易级 |
| 认知进化 | 辩证式 AI + 引用 | 复用 `ai_config`，新增 LLM 调用命令 |
| 可追溯 | 纪律分 | 由 `trade_discipline` + `trade_reviews` 聚合 |

---

## 2. 现状盘点：可复用资产与关键缺口

### 2.1 已存在、可直接复用（20 张表 / 90+ Tauri 命令）

| 现有资产 | 位置 | 风控用途 |
|---|---|---|
| `holdings`（含 `CASH_*` 虚拟现金持仓、`currency`） | db | **实时持仓真值**、总资产、集中度分母 |
| `cached_quotes`（22 行，含 `change_percent`） | db | 实时价格、止损判定 |
| `cached_exchange_rates`（usd_cny / usd_hkd / cny_hkd） | db | 多币种统一折算 |
| `HoldingDetail.market_value_usd` / `DashboardSummary.base_currency` | `models/dashboard.rs` | 归一化市值（**默认 USD**，非 CNY） |
| `daily_portfolio_values`（112 行 `total_value`） | db | 历史净值序列、TWR 基 |
| `snapshots::take_snapshot` / `backfill_snapshots` | commands | 每日净值落库触发点 |
| `performance_service.rs` 的 TWR / 最大回撤 / 波动率 / 夏普 | services | **复用 TWR 计算**，避免重复实现 |
| `price_alerts`（PRICE_ABOVE/BELOW、CHANGE_*、PNL_*） | db + `alert_service.rs` | 用户自定义阈值告警（**保留并存**） |
| `review::*`、`quarterly_holding_snapshots.decision_quality` | commands/services | 复盘与决策质量评分 |
| `transactions.notes` | db | 交易理由（`strategyNote` 等价物） |
| `ai_config`（provider / api_key / model / base_url / system_prompt） | db | AI 顾问配置（**目前仅配置，无调用**） |
| `categories` 多标签 | db | 集中度可按类别扩展 |
| 前端 `/alerts` `/review` `/quarterly` 页面 | src/pages | 风控 UI 的挂载点 |

### 2.2 关键缺口（grep 全库无结果）

- ❌ **观察列表 / 冷静期 / 风控规则 / HWM 水位线**：完全不存在
- ❌ **组合级自动风控**：现有 `price_alerts` 是「用户手动为单标的设阈值」，不是「系统自动计算组合级铁律」
- ❌ **持久化 HWM**：`calculate_max_drawdown` 是**区间内历史回撤**（画图表用），不是「实时水位线熔断」
- ❌ **AI 实际调用**：`commands/ai.rs` 只有 `get/update_ai_config`，没有任何 LLM 请求
- ❌ **交易前置校验**：`create_transaction` 直接写库，无任何纪律拦截
- ❌ **显式出/入金事件**：交易类型仅 `BUY / SELL / PAY / OPEN`，现金靠 `CASH_*` 持仓的 `shares` 表达

### 2.3 必须适配的差异（含排除理由）

| 参考系统做法 | 本项目决策 | 理由 |
|---|---|---|
| 前端实时计算风控 | **Rust 后端计算** | 本地 SQLite 是唯一真值源；后端计算可保证「录入前拦截」不可被前端绕过，且与 Tauri 命令层一致 |
| LocalStorage + Supabase 云同步 | **纯本地 SQLite** | 项目无云依赖、单端使用；参考系统「HWM 云端取 max 防覆盖」的冲突场景不存在 |
| 折算为 CNY | **沿用 `base_currency`（默认 USD），可配** | 项目现有仪表盘/统计均以 USD 为基准，强行改 CNY 会与所有既有数字不一致 |
| SHORT / COVER / DEPOSIT / WITHDRAW | **不照搬** | 现货交易仅 BUY/SELL/PAY/OPEN；做空在 `crypto_contract`（`position_type=long/short`）另有体系，应单独设合约规则 |
| Bloomberg / Cyberpunk 暗黑风 | **沿用 light 主题 + Ant Design** | 项目统一浅色极简风格；用户明确偏好简洁、删除冗余、精细标题梯度 |
| 邮件 9:15 推送 | **应用内预警 + OS 通知** | 桌面应用无常驻服务与邮件渠道，不引入外部依赖 |
| 只显示比例（隐私优先） | **默认显示金额，提供隐私开关** | 现有项目全面显示金额；保留参考理念但做成可选，不破坏既有习惯 |
| LlamaIndex Cloud RAG | **本地 SQLite 检索** | 本地优先、无 API 成本、数据不出本机；笔记量级（个人使用）无需向量库 |
| Node Cron 定时任务 | **应用启动 + 前端轮询 + 手动体检** | Tauri 无常驻后台进程；现有 `CryptoContract` 已有 30s 轮询行情的成熟模式 |

---

## 3. 总体架构

### 3.1 分层

```
┌──────────────────────────────────────────────────────────────┐
│  展示层 React + Ant Design (light)                            │
│  /risk 风控驾驶舱  │  /watchlist 观察列表  │  /review 复盘增强   │
│  交易录入弹窗：前置预检 → 风控结论 → BLOCK 需填覆盖理由          │
└───────────────────────────┬──────────────────────────────────┘
                            │ invoke (Tauri commands)
┌───────────────────────────▼──────────────────────────────────┐
│  编排层 commands/risk.rs                                      │
│  evaluate_risk / precheck_trade / acknowledge_event / ...     │
│  ┌────────────────────────────────────────────────────────┐  │
│  │ 拦截点：create_transaction 前置调用 precheck_trade      │  │
│  │ 返回 Verdict{PASS|WARN|BLOCK}，交易与留痕同一事务        │  │
│  └────────────────────────────────────────────────────────┘  │
├──────────────────────────────────────────────────────────────┤
│  领域层 services/risk_engine.rs（纯函数，可单测）              │
│  RiskContext → [RiskRule] → Vec<RiskFinding>                  │
│  规则：STOP_LOSS / MAX_DRAWDOWN / CONCENTRATION /             │
│        COOLING_PERIOD / UNPLANNED_TRADE / STALE_DATA         │
│  去重状态机：risk_events (active → acknowledged → resolved)   │
├──────────────────────────────────────────────────────────────┤
│  数据层 SQLite                                                │
│  【实时真值】holdings + cached_quotes + cached_exchange_rates │
│  【历史序列】daily_portfolio_values → TWR 单位净值 → HWM      │
│  【新增】risk_rules / risk_events / portfolio_hwm / watchlist │
│          trade_discipline / investment_memos / trade_reviews  │
│  【复用】transactions.notes / price_alerts / ai_config        │
└──────────────────────────────────────────────────────────────┘
```

### 3.2 三条核心数据流

**① 实时体检（只读，30s 或手动触发）**
```
holdings(实时) + cached_quotes + 汇率
   → build RiskContext（含 unit_nav / HWM / 权重 / 数据新鲜度）
   → 逐规则 evaluate() → Vec<RiskFinding>
   → 与 risk_events 比对去重/升级/恢复 → 落库
   → 前端驾驶舱渲染
```

**② 交易拦截（写入路径，强一致）**
```
交易录入 → precheck_trade(intent)
   → 查 watchlist / holdings / 冷静期天数
   → Verdict: PASS | WARN(可继续) | BLOCK(需 override_reason)
   → 通过后 create_transaction 与 trade_discipline 留痕【同一 SQLite 事务】
   → 触发一次增量体检（新仓位可能影响集中度/止损）
```

**③ 复盘闭环（T+30 / 手动）**
```
transactions（含 notes）→ LIFO 配对成 round-trip
   → 计算胜率/盈亏比/持有天数
   → due_at = traded_at + 30d 生成时光机待办
   → 用户回填 decision_quality + 复盘笔记
   → AI 辩证分析（可选）→ 引用留痕 → 聚合纪律分
```

### 3.3 设计原则

1. **实时真值优先**：风控判定只用 `holdings` + 行情缓存，**绝不用日/季度快照**（§5.5）
2. **规则与阈值分离**：阈值存 `risk_rules` 表，用户可调，代码无魔法数字
3. **纯函数引擎**：`evaluate(ctx) -> findings` 无 IO，便于单测与回放
4. **拦截不可绕过但可留痕**：允许强制覆盖（尊重成年人），但必须填理由并进入复盘
5. **卖出永不拦截**：止损/减仓是风控希望发生的行为，冷静期只约束买入
6. **降级优于误报**：数据过期/缺失时降级提示，不得用脏数据触发熔断

---

## 4. 数据模型设计

### 4.1 新增表（SQLite DDL）

```sql
-- ① 规则配置：阈值可改、可停用，代码不写死
CREATE TABLE IF NOT EXISTS risk_rules (
  id          TEXT PRIMARY KEY,            -- STOP_LOSS / MAX_DRAWDOWN / ...
  name        TEXT NOT NULL,
  description TEXT,
  severity    TEXT NOT NULL,               -- block | critical | warning | info
  enabled     INTEGER NOT NULL DEFAULT 1,
  threshold   REAL,                        -- 主阈值：-20 / -5 / 15 / 7
  params      TEXT,                        -- JSON 扩展参数
  updated_at  TEXT NOT NULL
);
INSERT OR IGNORE INTO risk_rules VALUES
  ('STOP_LOSS','止损红线',      '单笔持仓亏损超过阈值',        'critical',1,-20.0,NULL,datetime('now')),
  ('MAX_DRAWDOWN','最大回撤熔断','单位净值较 HWM 回撤超阈值',  'block',   1, -5.0,NULL,datetime('now')),
  ('CONCENTRATION','个股持仓上限','单标的占总资产超阈值',      'warning', 1, 15.0,NULL,datetime('now')),
  ('COOLING_PERIOD','7日冷静期', '观察期未满买入',             'block',   1,  7.0,NULL,datetime('now')),
  ('UNPLANNED_TRADE','非计划交易','既无持仓也不在观察列表',    'warning', 1, NULL,NULL,datetime('now')),
  ('STALE_DATA','行情陈旧',      '行情超过 N 分钟未更新',      'info',    1, 60.0,NULL,datetime('now'));

-- ② 风控事件：支持去重、确认、解除，带触发时上下文快照
CREATE TABLE IF NOT EXISTS risk_events (
  id                 TEXT PRIMARY KEY,
  rule_id            TEXT NOT NULL,
  symbol             TEXT,                  -- NULL = 组合级（如回撤熔断）
  account_id         TEXT,
  severity           TEXT NOT NULL,
  status             TEXT NOT NULL,         -- active | acknowledged | resolved
  metric_value       REAL,                  -- 触发时实际值
  threshold          REAL,
  message            TEXT NOT NULL,
  context            TEXT,                  -- JSON：当时价格/净值/权重/数据时间戳
  first_triggered_at TEXT NOT NULL,
  last_triggered_at  TEXT NOT NULL,
  resolved_at        TEXT,
  FOREIGN KEY(rule_id) REFERENCES risk_rules(id)
);
CREATE INDEX IF NOT EXISTS idx_risk_events_active
  ON risk_events(status, rule_id, symbol);
-- 去重唯一键：同一规则+标的+组合维度下仅保留一条 active
CREATE UNIQUE INDEX IF NOT EXISTS idx_risk_events_dedup
  ON risk_events(rule_id, IFNULL(symbol,'*'), IFNULL(account_id,'*'))
  WHERE status = 'active';

-- ③ 历史最高水位：基于 TWR 单位净值，支持 GLOBAL / 单账户
CREATE TABLE IF NOT EXISTS portfolio_hwm (
  scope_key        TEXT PRIMARY KEY,       -- 'GLOBAL' 或 account_id
  peak_unit_nav    REAL NOT NULL,
  peak_date        TEXT NOT NULL,
  current_unit_nav REAL NOT NULL,
  updated_at       TEXT NOT NULL
);

-- ④ 观察列表：冷静期起点 + 投资逻辑
CREATE TABLE IF NOT EXISTS watchlist (
  id           TEXT PRIMARY KEY,
  symbol       TEXT NOT NULL,
  name         TEXT,
  market       TEXT NOT NULL,
  currency     TEXT,
  added_at     TEXT NOT NULL,              -- 冷静期起算
  thesis       TEXT,                       -- 关注理由（买入前必须想清楚）
  target_price REAL,
  status       TEXT NOT NULL DEFAULT 'watching',  -- watching | promoted | dropped
  promoted_at  TEXT,
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL,
  UNIQUE(symbol, market)
);
CREATE INDEX IF NOT EXISTS idx_watchlist_added ON watchlist(added_at);

-- ⑤ 交易纪律留痕：每次拦截/覆盖都留证据，供纪律分与复盘
CREATE TABLE IF NOT EXISTS trade_discipline (
  id               TEXT PRIMARY KEY,
  transaction_id   TEXT,                   -- 最终成交 ID（若成交）
  symbol           TEXT NOT NULL,
  action           TEXT NOT NULL,          -- BUY | SELL
  verdict          TEXT NOT NULL,          -- pass | warned | blocked | override
  rule_ids         TEXT,                   -- JSON array
  cooling_days     REAL,
  is_unplanned     INTEGER NOT NULL DEFAULT 0,
  override_reason  TEXT,                   -- BLOCK 被强制放行时必填
  decided_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_trade_discipline_symbol ON trade_discipline(symbol, decided_at);

-- ⑥ 投资笔记：创建时冻结持仓快照
CREATE TABLE IF NOT EXISTS investment_memos (
  id                TEXT PRIMARY KEY,
  title             TEXT NOT NULL,
  content           TEXT NOT NULL,         -- Markdown
  tags              TEXT,                  -- 逗号分隔
  holdings_snapshot TEXT,                  -- JSON：当时持仓/权重/净值/HWM
  created_at        TEXT NOT NULL,
  updated_at        TEXT NOT NULL
);

-- ⑦ 复盘与 AI 分析
CREATE TABLE IF NOT EXISTS trade_reviews (
  id               TEXT PRIMARY KEY,
  transaction_id   TEXT,
  symbol           TEXT NOT NULL,
  review_type      TEXT NOT NULL,          -- time_capsule | ai_analysis | manual
  decision_quality TEXT,                   -- correct | wrong | pending
  content          TEXT,
  citations        TEXT,                   -- JSON 数组
  due_at           TEXT,                   -- traded_at + 30 天
  completed_at     TEXT,
  created_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_trade_reviews_due ON trade_reviews(due_at, completed_at);

-- ⑧ 外部现金流（前置依赖，见 §7.3）
CREATE TABLE IF NOT EXISTS external_cash_flows (
  id           TEXT PRIMARY KEY,
  account_id   TEXT NOT NULL,
  currency     TEXT NOT NULL,
  flow_type    TEXT NOT NULL,              -- DEPOSIT | WITHDRAW | DIVIDEND_ADJUST
  amount       REAL NOT NULL,              -- 入金为正、出金为负
  occurred_at  TEXT NOT NULL,
  note         TEXT,
  created_at   TEXT NOT NULL
);
```

### 4.2 复用字段映射

| 参考系统字段 | 本项目对应 | 说明 |
|---|---|---|
| `Transaction.strategyNote` | `transactions.notes` | 已存在，交易理由 |
| `PortfolioRiskState.highWaterMark` | `portfolio_hwm.peak_unit_nav` | **改为单位净值**（§7） |
| `positions[].pnl_percent` | `HoldingDetail.pnl_percent` | 已有计算 |
| `allocation` | `DashboardSummary` + `holdings` | 现金为 `CASH_*` 虚拟持仓 |
| `watchlist` | 新增表 | 原项目无 |
| `asset_snapshots` | `daily_portfolio_values` | 已存在（112 行） |
| `investment_memos.holdingsSnapshot` | 新增表同名字段 | 参考实现照搬 |
| `trade_analysis_logs.citations` | `trade_reviews.citations` | 参考实现照搬 |
| `decision_quality` | 已有（季度级）+ 新增（交易级） | 理念下沉 |

### 4.3 迁移策略

- 所有新表用 `CREATE TABLE IF NOT EXISTS`，写在 `db/mod.rs` 的 migration 段（与现有 `decision_quality` 迁移同风格）
- `risk_rules` 用 `INSERT OR IGNORE` 播种，**不覆盖用户已调整的阈值**
- `portfolio_hwm` 首次初始化：从 `daily_portfolio_values` 回放 TWR 序列取历史峰值，避免上线首日 HWM 从 1.0 起算导致误报
- 全部为新增表，**不改动任何现有表结构**（零回归风险）

---

## 5. 风控规则引擎

### 5.1 领域模型（Rust）

```rust
// services/risk_engine.rs —— 纯函数，无 IO
pub enum Severity { Info, Warning, Critical, Block }

pub struct RiskPosition {
    pub symbol: String,
    pub market: String,
    pub account_id: String,
    pub shares: f64,
    pub avg_cost: f64,
    pub price: f64,
    pub market_value_base: f64,   // 折算到 base_currency
    pub pnl_percent: f64,
    pub quote_age_secs: i64,      // 数据新鲜度
}

pub struct RiskContext {
    pub as_of: DateTime<Utc>,
    pub base_currency: String,
    pub positions: Vec<RiskPosition>,      // 来自 holdings（实时）
    pub total_assets_base: f64,            // 含现金
    pub unit_nav: f64,                     // TWR 归一化
    pub hwm: f64,
    pub watchlist: HashMap<String, WatchItem>,
    pub rules: HashMap<String, RiskRuleConfig>,
}

pub struct RiskFinding {
    pub rule_id: String,
    pub severity: Severity,
    pub symbol: Option<String>,
    pub metric: f64,
    pub threshold: f64,
    pub message: String,
    pub context: serde_json::Value,        // 可追溯
}

pub trait RiskRule {
    fn id(&self) -> &str;
    fn evaluate(&self, ctx: &RiskContext) -> Vec<RiskFinding>;
}

pub fn evaluate_all(ctx: &RiskContext) -> Vec<RiskFinding>;  // 注册表中所有 enabled 规则
```

### 5.2 三大铁律的计算口径（**重点，避免口径错误**）

| 规则 | 公式 | 数据源 | 排除项 |
|---|---|---|---|
| **止损红线** | `pnl_pct = (price − avg_cost)/avg_cost × 100 ≤ −20%` | `holdings.avg_cost` + `cached_quotes.current_price` | 排除 `CASH_*` 现金持仓、排除 `shares ≤ 0`、排除加密合约（单列） |
| **回撤熔断** | `dd = (unit_nav − hwm)/hwm × 100 ≤ −5%` | `portfolio_hwm` + TWR 序列 | 排除外部现金流（TWR 已处理） |
| **集中度** | `weight = market_value_base / total_assets_base × 100 > 15%` | 折算后的 `market_value_base` | 分母**含现金**；分子排除现金；多账户按 `account_id` 或 GLOBAL 聚合 |

三个口径都必须**统一折算到 `base_currency`**（默认 USD）。项目已有 `HoldingDetail.market_value_usd` 与 `exchange_rates::convert_amount`，直接复用，不重新实现汇率逻辑。

**短仓说明**：现货模块无做空；`crypto_contract` 的 `position_type='short'` 建议单独设 `CONTRACT_LIQUIDATION_DISTANCE` 规则（现价与强平价距离），不混进现货止损口径（P4）。

### 5.3 分级处置

| Severity | UI 表现 | 行为 |
|---|---|---|
| `Block` | 红色实心横幅 + 拦截弹窗 | 交易预检直接拒绝；组合级熔断时全站顶部常驻 |
| `Critical` | 红色脉冲（参考系统的「高危警报」） | 建议立即斩仓 / 降总仓位，需手动 acknowledge |
| `Warning` | 橙色提示 | 记入预警中心，不阻断 |
| `Info` | 灰色说明 | 如「行情数据已 62 分钟未更新，结果可能滞后」 |

### 5.4 去重与状态机

```
无事件 + 命中规则  → INSERT risk_events(status=active)，记 first_triggered_at
active + 再次命中  → 更新 last_triggered_at / metric_value；若 severity 升级则重开提醒
active + 不再命中  → UPDATE status=resolved, resolved_at=now（自动解除）
active + 用户确认  → status=acknowledged（不再弹窗，但仍计入纪律统计）
```

唯一索引 `idx_risk_events_dedup` 保证「同一规则+标的+账户」只有一条 active，避免 30s 轮询刷屏。

### 5.5 ⚠️ 数据铁律：禁止用日快照判定持仓

**这是本次会话刚付出代价的教训，必须写进设计约束。**

同一个 `performance_service.rs` 里刚修复过两个生产 bug：

| Bug | 现象 | 根因 |
|---|---|---|
| HTT 清仓后显示大幅盈利 +366 | 实际亏损 −56 | `daily_holding_snapshots` 停在清仓**前**（市值 422），而卖出现金流已入账，卖出所得被重复计为收益 |
| RDDT 新建仓显示亏损 −300 | 实际 0 | 当日**尚无快照**，期末市值被当作 0，买入额被算成亏损 |

因此：

1. **风控引擎的持仓真值只能是 `holdings` 表**（它反映最新成交，HTT 清仓后 `shares=0`、RDDT 建仓后立即可见）
2. `daily_holding_snapshots` / `quarterly_holding_snapshots` 仅用于**历史曲线与归因**，绝不参与实时风控判定
3. 必须有 `STALE_DATA` 规则：行情 `quote_age_secs` 超阈值 → 相关指标**降级为 Info** 并标注「数据可能滞后」，不得用陈旧价格触发止损/熔断
4. **无行情 ≠ 价格为 0**：缺失行情的标的跳过止损/集中度判定并提示，不能因取到 0 而误判为腰斩

---

## 6. 交易前置拦截（防 FOMO）

### 6.1 决策矩阵

对 `BUY` 类交易（`SELL` 全部放行，见 §6.4）：

| 标的状态 | 观察期 | Verdict | 处理 |
|---|---|---|---|
| 已在 `holdings` 持仓 | — | `PASS` | 加仓自由（不惩罚已有仓位） |
| 在 `watchlist`，已满 7 天 | ≥7d | `PASS` | 记录 `cooling_days` |
| 在 `watchlist`，未满 7 天 | <7d | `BLOCK` | 弹「观察期未满 X 天，是否冲动交易？」；可强制覆盖 |
| 不在持仓也不在观察列表 | — | `WARN` | 标记「突发性/非计划交易」，需二次确认 |
| 组合处于 `MAX_DRAWDOWN=Block` | — | `BLOCK` | 熔断期间禁止新开仓（可覆盖） |

### 6.2 拦截流程（强一致）

```rust
// commands/risk.rs
pub fn precheck_trade(db, intent: TradeIntent) -> Result<Verdict, String>;
//  1. 组装轻量 RiskContext（含 watchlist / hwm / 熔断状态）
//  2. 执行 COOLING_PERIOD / UNPLANNED_TRADE / MAX_DRAWDOWN 规则
//  3. 返回 Verdict { level, findings[], cooling_days, requires_reason }

// commands/transactions.rs::create_transaction 增加：
//  if let Some(override_reason) = ... { 校验非空 }
//  在同一 SQLite 事务内：
//      INSERT transactions
//      INSERT trade_discipline (verdict / rule_ids / override_reason)
//      UPDATE watchlist.status='promoted'（若来自观察列表）
//  事务提交后触发一次增量 evaluate_risk
```

**关键**：`transactions` 与 `trade_discipline` 必须在**同一事务**，否则会出现「成交了但没留痕」，纪律分与复盘就会失真。

### 6.3 覆盖机制（尊重成年人，但必须留痕）

- 允许「仍然买入」，但 `override_reason` 为必填文本
- 该笔交易自动打上 `is_unplanned/override` 标记，进入 T+30 时光机**优先复盘队列**
- 驾驶舱展示「冷静期遵守率」，形成温和的行为反馈，而非硬性禁止

### 6.4 为什么卖出永不拦截

风控目标是「活下来 + 少犯错」。拦截卖出会妨碍止损与减仓，与三大铁律直接冲突。所有冷静期/非计划规则只作用于 `BUY`；`SELL` 仅记录、不阻断。这是参考材料未明说但必须补上的设计约束。

---

## 7. HWM 与净值体系（对参考实现的关键修正）

### 7.1 参考实现的缺陷

参考系统用 `PortfolioRiskState.highWaterMark`（**总净值峰值**）判断回撤：

```
drawdown = (totalNetWorthCNY − HWM) / HWM
```

问题：总净值会被**非投资行为**污染——

- **入金** → 净值跳升，HWM 被抬高，真实亏损被掩盖
- **出金** → 净值跳水，`drawdown` 虚假击穿 −5%，触发无意义熔断

参考材料自己也意识到这点，所以在云端同步时特意做了「HWM 取 max 防止被覆盖」，但那是同步冲突问题，没解决口径问题。

### 7.2 本项目方案：TWR 单位净值

复用 `performance_service.rs` 已有的 TWR 能力（该文件含 `test_twr_with_cash_flow` 等测试）：

```
单位净值 u(t) = u(t−1) × (1 + r_t)
r_t = (V_t − F_t − V_{t−1}) / V_{t−1}
  V_t   = 期末总资产（holdings 市值 + 现金，折算 base_currency）
  F_t   = 期间净外部现金流（入金为正、出金为负）
u(0)  = 1.0（或 100）
HWM   = max(u(t)) 全历史
当前回撤 = (u(now) − HWM) / HWM
```

**好处**：买卖只是资产内部形态转换（现金↔持仓），不改变单位净值；只有真实盈亏与出入金影响它，而 F_t 又把出入金剔除了 → 回撤纯粹反映投资决策质量。

`portfolio_hwm` 存 `peak_unit_nav / peak_date / current_unit_nav`，每次快照或体检时更新（只增不减，除非用户显式「校准重置」）。

### 7.3 ⚠️ 前置依赖：项目缺少显式出入金事件

当前交易类型只有 `BUY / SELL / PAY / OPEN`，现金以 `CASH_USD` 等虚拟持仓的 `shares` 表达；`OPEN` 是初始持仓录入（不影响现金）。**没有 DEPOSIT / WITHDRAW 事件**，导致 F_t 无法准确取得。

两个方案：

| 方案 | 做法 | 评价 |
|---|---|---|
| **A（推荐）** | 新增 `external_cash_flows` 表 + 设置页「入金/出金」入口；`take_snapshot` 时把期间 F_t 写进 TWR | 口径精确，且顺带补齐资产校准能力；约 1 张表 + 2 个命令 |
| B（降级） | 在 `daily_portfolio_values` 新增 `external_flow` 列，通过现金持仓的异常变动推断 | 实现快但脆弱，容易把大额买卖误判为出入金 |

**建议 P0 先做 A 的表结构**（哪怕 UI 后补），否则 HWM 熔断在出入金时必然误报。这一点需要你确认：是否接受新增「入金/出金」记录入口。

### 7.4 校准与重置

- 提供「资产校准」：录入实际现金差额，写入 `external_cash_flows`（类型 `DIVIDEND_ADJUST`）
- 重置 HWM 需二次确认，并在 `risk_events` 留一条审计事件——防止「回撤难看就重置水位」这一人性漏洞（很符合参考系统的偏执精神）

---

## 8. 复盘与「过程即奖励」

### 8.1 LIFO 完整交易配对

基于 `transactions` 按 `(account_id, symbol)` 分组、时间排序，用**后进先出**匹配 BUY/SELL 生成 round-trip：

```rust
pub struct RoundTrip {
    pub symbol: String,
    pub open_at: String,
    pub close_at: String,
    pub holding_days: i64,
    pub qty: f64,
    pub avg_open_price: f64,
    pub avg_close_price: f64,
    pub realized_pnl: f64,      // 扣除 commission
    pub return_rate: f64,
    pub open_note: Option<String>,   // transactions.notes
}
```

聚合指标：胜率、盈亏比（平均盈利/平均亏损）、总已结盈亏、平均持有天数、最佳/最差交易。
**注意**：部分卖出需按 LIFO 拆分；`OPEN` 作为期初建仓参与配对但不产生现金流。

### 8.2 时光机（T+30 强制复盘）

- 每笔成交写入时生成 `trade_reviews(review_type='time_capsule', due_at = traded_at + 30d)`
- 驾驶舱/复盘页顶部常驻「待复盘 N 笔」，展示当初的 `notes`（买入理由）与**当时的持仓快照**
- 用户回填 `decision_quality ∈ {correct, wrong, pending}` + 复盘文字：当初逻辑是否兑现？是否属于冲动交易？
- 被 `override` / `is_unplanned` 标记的交易**优先展示**（正好检验「那次破例值不值」）

### 8.3 纪律分（Discipline Score）

把「过程」量化，作为对收益之外的第二评价轴：

| 分项 | 权重 | 计算 |
|---|---|---|
| 冷静期遵守率 | 30% | `1 − override次数 / 新建仓次数` |
| 计划交易率 | 25% | `1 − 非计划交易数 / BUY 数` |
| 止损执行率 | 25% | 触发 `STOP_LOSS` 后 N 日内减/清仓的比例 |
| 复盘完成率 | 20% | 已完成 / 到期时光机数 |

分数只作趋势展示，**不设排行榜、不做惩罚**——符合「过程即奖励」的自我对照定位，避免把系统变成另一个焦虑源。

### 8.4 投资笔记 + 持仓快照

- `investment_memos` 创建时冻结 `holdings_snapshot`（持仓、权重、净值、HWM、当时触发的风控事件）
- 支持 Markdown + 标签；详情页并排展示「当时怎么想」与「后来怎么样」
- 复用 `quarterly_holding_snapshots.decision_quality` 的既有理念，把它从季度级下沉到交易级

---

## 9. AI 辩证式顾问

### 9.1 定位

**不给买卖建议，只挑战假设。** 参考系统 PIP 的核心提示词要求「辩论式反馈、主动挑战用户假设、指出潜在盲点」，并要求每个观点带内联引用。这比「AI 荐股」更契合风控目标。

### 9.2 复用与替代

| 参考实现 | 本项目 |
|---|---|
| Gemini 3 Pro + LlamaIndex Cloud 4 路 RAG | 复用 `ai_config`（provider / model / base_url / api_key / system_prompt），HTTP 直连；**本地检索替代云 RAG** |
| 风和亚洲哲学 / 播客 / 笔记 / 动态笔记 四知识源 | 本地 `investment_memos` + `transactions.notes` + `risk_rules` + `trade_reviews` + `watchlist.thesis`，SQLite FTS5 或关键词评分 |
| 云 Pipeline ID | 无需；数据不出本机 |

新增 `user_profile` 可选表（性格画像 / 投资原则 / 认知偏差），对应参考系统的「个性化档案」；首版可先用 `ai_config.system_prompt` 承载，避免过度设计。

### 9.3 输出契约（结构化 JSON，便于落库与渲染）

```json
{
  "verdict": "challenge",
  "challenges": [
    {"claim": "该笔买入属于观察期未满的破例", "basis": "COOLING_PERIOD 规则命中", "citation": {"source": "risk_event", "ref": "evt_xxx"}}
  ],
  "principle_conflicts": [{"principle": "5万以下不做T", "evidence": "memo_2026xxxx"}],
  "risk_notes": ["当前组合回撤 -4.2%，接近 -5% 熔断线"],
  "suggestions": ["先记录本次破例理由，T+30 强制复盘"],
  "citations": [{"source": "investment_memos", "excerpt": "...", "ref": "memo_xxx"}]
}
```

### 9.4 系统提示词要点

- 角色：个性化投资伙伴，**义务是质疑而非附和**
- 必须引用本地可追溯来源（笔记 / 规则 / 历史交易 / 用户原则）
- 明确禁止给出「买/卖/目标价」类指令
- 结合 `decision_quality` 历史，提醒重复出现的认知偏差
- API Key 沿用 `ai_config`（当前明文存本地 SQLite；若后续同步需加密，列为 P4 事项）

---

## 10. 前端设计

### 10.1 新增入口

| 路由 | 页面 | 内容 |
|---|---|---|
| `/risk` | **风控驾驶舱** | 三铁律状态卡、HWM 回撤仪表、预警中心、纪律分、数据新鲜度 |
| `/watchlist` | **观察列表** | 关注标的、冷静期倒计时、投资逻辑（thesis）、目标价、转持仓 |
| `/review`（增强） | 操作复盘 | 时光机待办、LIFO round-trip、决策质量回填、AI 分析 |
| 交易录入弹窗 | 前置预检 | Verdict 展示、BLOCK 时覆盖理由输入 |
| 设置页 | 风控设置 | 阈值调整、规则启停、观察期天数、base_currency、隐私开关、HWM 校准 |

### 10.2 驾驶舱信息层级（遵循极简偏好）

```
[全站顶部] 仅 Block 级熔断时出现的常驻横幅
┌────────────────────────────────────────────────┐
│ 风控状态  ● 正常 / ⚠ 预警 / ⛔ 熔断             │  ← 单一状态，不堆小字
├──────────────┬──────────────┬──────────────────┤
│ 当前回撤     │ 止损持仓      │ 最大集中度        │  ← 三铁律，数字+迷你进度条
│ -2.1% / -5%  │ 1 / 15%      │ PBR.A 18.3%      │
├────────────────────────────────────────────────┤
│ 待处理预警（按 severity 排序，可 acknowledge）   │
├────────────────────────────────────────────────┤
│ 纪律分 82  │  待复盘 3 笔  │  观察中 5 标的     │
└────────────────────────────────────────────────┘
```

- **删除冗余提示文字**，标题层级压缩，只用两级（页面标题 / 卡片标题）
- 涨红跌绿（中国习惯），与项目现有 `#E53935 / #43A047` 保持一致
- **不引入暗黑 Bloomberg 风**，沿用 light + Ant Design
- 行情陈旧时卡片显示灰色角标，不用弹窗打扰

### 10.3 与现有页面关系

- `/alerts` **保留**：那是用户自定义阈值提醒；`/risk` 是系统自动铁律，两者语义不同、并存
- `/performance` 的 `DrawdownChart` / `RiskMetricsPanel` 保留（历史分析），`/risk` 专注**实时状态与处置**
- `/quarterly` 的 `decision_quality` 保留，与交易级复盘形成季度↔单笔两层视角

---

## 11. 自动化与数据质量

### 11.1 触发机制（Tauri 无常驻 cron）

| 时机 | 动作 |
|---|---|
| 应用启动 | 全量 `evaluate_risk` + 时光机到期扫描 + 净值快照补齐 |
| 前端 30s 轮询 | 复用 `CryptoContract` 现有行情轮询模式，顺带刷新风控状态 |
| 行情刷新后 | 增量体检（只重算受影响标的） |
| 交易提交后 | 强制增量体检 |
| 每日首次启动 | 调用 `snapshots::take_snapshot` → 更新 TWR / HWM |
| 手动 | 驾驶舱「立即体检」按钮 |

不引入邮件；Block 级可用 Tauri 系统通知插件（可选 P4）。

### 11.2 数据质量校验（参考材料 §数据保护机制）

在 `take_snapshot` 与行情写入前加防护，避免脏数据触发误熔断：

| 校验 | 阈值 | 动作 |
|---|---|---|
| 单标的价格变化 | > 95% | 拒绝写入，记 `STALE_DATA` 事件 |
| 首次价格范围 | 0.01 ~ 100000 | 拒绝 |
| USD/CNY 汇率 | 5 ~ 10 | 拒绝 |
| HKD/CNY 汇率 | 0.7 ~ 1.2 | 拒绝 |
| 汇率日变动 | > 20% | 拒绝 |
| 单位净值日变动 | > 95% | 拒绝保存快照（防 TWR 断崖） |

### 11.3 压力测试（参考系统亮点，P4）

持仓明细支持临时改价（只影响内存预览、不落库）：「如果 PBR.A 跌 30%，回撤会到多少？会否触发熔断？」——复用 `RiskContext` 的纯函数特性，构造一个假行情上下文直接 evaluate 即可，实现成本很低。

---

## 12. 接口契约（新增 Tauri commands）

```
risk::get_risk_dashboard()                  -> RiskDashboard   // 三铁律 + 事件 + 纪律分 + 新鲜度
risk::evaluate_risk()                       -> Vec<RiskFinding>// 手动全量体检
risk::acknowledge_event(id)                 -> ()
risk::get_risk_rules()                      -> Vec<RiskRuleConfig>
risk::update_risk_rule(id, enabled, threshold, params) -> ()
risk::get_hwm()                             -> HwmState
risk::recalibrate_hwm(reason)               -> ()              // 需二次确认 + 审计
risk::stress_test(overrides)                -> RiskDashboard   // P4

watchlist::list_watchlist()                 -> Vec<WatchItem>
watchlist::add_to_watchlist(symbol, thesis, target_price) -> WatchItem
watchlist::update_watchlist(id, ...)        -> ()
watchlist::remove_from_watchlist(id)        -> ()

risk::precheck_trade(intent)                -> Verdict         // 交易前置
discipline::list_discipline_logs(filter)    -> Vec<DisciplineLog>
discipline::get_discipline_score()          -> DisciplineScore

memos::list_memos() / create_memo() / update_memo() / delete_memo()
reviews::list_due_reviews() / complete_review(id, quality, content)
reviews::get_round_trips(symbol?)           -> Vec<RoundTrip>
ai::analyze_trade(transaction_id)           -> TradeAnalysis   // 复用 ai_config
ai::chat(message, conversation_id)          -> AiReply         // P3

cashflow::record_external_flow(...)         -> ()              // §7.3 方案 A
```

所有命令遵循项目现有 `#[tauri::command(rename_all = "camelCase")]` 与 `Result<T, String>` 风格，在 `lib.rs::invoke_handler` 注册。

---

## 13. 实施路线

| 阶段 | 交付 | 依赖 | 验收标准 |
|---|---|---|---|
| **P0 风控引擎** | 8 张表 + `risk_engine.rs` + 三铁律 + TWR/HWM + `/risk` 驾驶舱 + 数据新鲜度 | **确认 §7.3 出入金方案** | 用真实库回放：HTT 已清仓不触发止损、RDDT 新建仓不显示亏损；三铁律数值与手工核算一致 |
| **P1 防 FOMO** | `watchlist` + `precheck_trade` + 交易弹窗拦截 + 留痕 | P0 | 观察期未满的 BUY 被 BLOCK；SELL 永不拦截；覆盖必填理由且入 `trade_discipline` |
| **P2 复盘** | LIFO round-trip + 时光机 T+30 + 纪律分 + 投资笔记(持仓快照) | P1 | 24 笔历史交易能正确配对；到期复盘可回填 `decision_quality` |
| **P3 AI 顾问** | `ai::analyze_trade` / `chat` + 本地检索 + 引用渲染 | P2、有效 API Key | 输出含可追溯引用、不出现买卖指令 |
| **P4 增强** | 数据质量校验、压力测试、合约风控规则、OS 通知、API Key 加密 | P0–P3 | 脏汇率/脏价格被拒绝且不误触熔断 |

**每阶段独立可用**：P0 完成即有价值（实时风控体检），后续阶段是增量叠加，不需要「全部做完才能用」。

---

## 14. 风险、边界与待确认

| # | 事项 | 说明 / 需你决策 |
|---|---|---|
| 1 | **出入金事件缺失** | TWR/HWM 精度的前置依赖。是否接受新增「入金/出金」记录入口（方案 A）？若否，HWM 在出入金时可能误报，需降级方案 |
| 2 | **阈值个人化** | −20% / −5% / 15% / 7 天是参考默认值。你的实际风险偏好可能不同（例如加密仓位波动更大），建议上线后按历史回放调参 |
| 3 | **加密合约是否纳入** | P0 建议只覆盖现货 `holdings`；`crypto_contract`（含 1x 类现货、杠杆多空）口径不同，宜单设「强平距离 / 杠杆上限」规则 |
| 4 | **多账户 HWM** | `portfolio_hwm.scope_key` 已预留 `account_id`；当前 `accounts` 表 1 行，先按 GLOBAL 实现 |
| 5 | **行情源稳定性** | Gate.io/Binance、雪球/东财均可能被网络策略影响；`STALE_DATA` 是必要兜底，不能假设行情永远可用 |
| 6 | **API Key 明文** | `ai_config.api_key` 当前明文存本地库；单机可接受，若将来加云同步必须先加密 |
| 7 | **快照类问题的泛化** | 除 `performance_service.rs` 外，`get_return_attribution` 已有 shares=0 保护，但任何新模块都应遵守 §5.5 数据铁律 |
| 8 | **不追求「全自动」** | 参考系统偏执到「必须复盘」，本项目采取「允许覆盖但必须留痕」的温和策略，避免工具变成负担 |

---

## 15. 证据与出处

**参考材料**（`C:\Users\七七\Desktop\all_documents.zip`，6 个文件，均已逐份读取）：

| 文件 | 本设计引用内容 |
|---|---|
| `FullStack_Risk_System.ini` | 三大风控铁律、防 FOMO 7 日冷静期、HWM、LIFO、时光机、混合行情、云端 HWM 取 max、数据校验、「只有偏执狂才能生存」 |
| `RiskControl_Prompt.ini` | 规则阈值（−20% / −5% / 15%）、`PortfolioRiskState` / `Transaction` 数据结构、交易类型枚举、LIFO 统计指标 |
| `Investment_Mirror_Intro.md` | 风和亚洲投资哲学作为知识源、辩证式反馈、有据可查、性格画像/认知偏差、引用展示、动态笔记 |
| `Manus_Build_Guide.md` | PIP 系统提示词要求（辩论式、挑战假设、内联引用）、RAG 四源、系统提示词结构、AI 输出与引用格式 |
| `AI_Advisor_Prompt.md` | 每日 9:15 交易分析、`strategyNote` 交易理由、投资笔记+持仓快照、数据保护机制（价格/汇率/净值校验）、定时任务 |
| `Process_As_Reward.md` | 「过程即奖励」理念、隐私优先、投资笔记、AI 照镜子分析维度、时光机式复盘、数据校验阈值 |

**项目现状**（本地代码与数据库实测，2026-11-07）：

| 来源 | 核实内容 |
|---|---|
| `data/com.portfolio.manager/portfolio.db` | 20 张表结构、行数；`holdings`/`transactions`/`price_alerts`/`ai_config`/`cached_exchange_rates`/`daily_portfolio_values`/`quarterly_holding_snapshots` 字段 |
| `src-tauri/src/lib.rs` | 已注册 90+ Tauri 命令清单（确认无风控/观察列表/AI 调用命令） |
| `src-tauri/src/services/performance_service.rs` | TWR 能力、`calculate_max_drawdown` 为区间回撤（非 HWM）、`get_return_attribution` 的 shares=0 保护 |
| `src-tauri/src/commands/transactions.rs` | 交易类型 `BUY/SELL/PAY/OPEN`、`cash_delta`、`adjust_cash_holding`、`CASH_*` 虚拟现金持仓 |
| `src-tauri/src/services/alert_service.rs` | 现有告警为手动阈值（PRICE_*/CHANGE_*/PNL_*），非组合级自动规则 |
| `src-tauri/src/models/dashboard.rs` | `DashboardSummary.base_currency`、`HoldingDetail.market_value_usd` |
| `src-tauri/src/services/snapshot_service.rs` | `daily_portfolio_values.total_value` 生成逻辑 |
| `src/components/Layout/MainLayout.tsx` | 现有 14 个导航入口 |
| 本次会话已修复的生产 bug | HTT 清仓后陈旧快照导致虚假盈利（`c3d4306`）、RDDT 新建仓无快照导致虚假亏损（`0ddeb87`）——§5.5 数据铁律的直接依据 |

---

---

## 16. P0 实施记录（2026-11-07）

### 16.1 已确认决策

| # | 待确认项 | 决策 |
|---|---|---|
| 1 | 出入金事件缺失 | **方案 A**：新增 `external_cash_flows` 表 + 驾驶舱「出入金」录入入口，记录与现金持仓调整在同一 SQLite 事务 |
| 2 | 阈值个人化 | 沿用参考默认：止损 −20% / 回撤 −5% / 集中度 15% / 冷静期 7 天（后两者 P1 起生效），UI 可调 |
| 3 | 加密合约 | **不并入三铁律**；已播种 `CONTRACT_LIQUIDATION_DISTANCE`（强平距离 20%，默认停用），P4 启用 |
| 4 | 多账户 HWM | 先按 `scope_key='GLOBAL'` 实现 |
| 5 | 基准货币 | 固定 USD（与仪表盘/统计一致），`base_currency` 暂不暴露为可配 |

### 16.2 已交付文件

| 层 | 文件 | 内容 |
|---|---|---|
| 迁移 | `src-tauri/src/db/mod.rs` | `migrate_risk_control_tables`：8 张新表 + 7 条规则播种 + 幂等列升级；**未改动任何现有表** |
| 模型 | `src-tauri/src/models/risk.rs` | `RiskRuleConfig / RiskEvent / HwmState / ExternalCashFlow / RiskFinding / RiskPosition / RiskContext / QuoteFreshness / RiskDashboard` |
| 引擎 | `src-tauri/src/services/risk_engine.rs` | 纯函数 `evaluate_all`：STOP_LOSS / MAX_DRAWDOWN / CONCENTRATION / STALE_DATA，含 4 个单测 |
| 编排 | `src-tauri/src/services/risk_service.rs` | 实时上下文构建、单位净值与 HWM、事件去重状态机、驾驶舱聚合、规则配置、HWM 校准审计 |
| 出入金 | `src-tauri/src/services/cashflow_service.rs` | 记录/列表/删除，记录与 `$CASH-*` 调整同事务 |
| 命令 | `commands/risk.rs`、`commands/cashflow.rs` | 10 个 Tauri 命令，已在 `lib.rs::invoke_handler` 注册 |
| 前端 | `src/pages/Risk/index.tsx`、`src/stores/riskStore.ts`、`src/types/index.ts` | 驾驶舱：三铁律状态卡、预警中心、规则设置、出入金、净值基准；30s 自动体检 |
| 集成 | `src/App.tsx`、`src/components/Layout/MainLayout.tsx` | `/risk` 路由 + 「风控中心」导航 |

### 16.3 与原设计的偏差（均有实测依据）

| # | 原设计 | 实施 | 理由 |
|---|---|---|---|
| 1 | HWM 从 `daily_portfolio_values` 回放历史 TWR 序列 | **自启用日起算**（`base_date`），历史快照不参与熔断 | 实测 `daily_portfolio_values.total_value` 用 `shares > 0`，**排除了现金持仓**，买卖会让它凭空跳变；若强行回放会制造大量假回撤。宁可少一段历史，也不能用口径不一致的数据触发熔断 |
| 2 | 逐日 TWR 链 `u(t)=u(t−1)×(1+r_t)` | 简化单位净值法 `u=(V−ΣF)/V₀` | 首版无逐日估值+现金流时点；该式同样能剔除出入金污染（见 §16.4 验证），P4 可升级为逐日链 |
| 3 | `V(t)` = 持仓市值 + 现金 | **显式计入 `$CASH-*` 现金持仓（允许为负）** | 实测现金是虚拟持仓且当前为 −329.60 USD；若排除则买入会让 V 增加，TWR 把买入误判为收益 |
| 4 | `STALE_DATA` 60 分钟 | **1440 分钟（24h）** | 本项目是日频持仓管理，60 分钟会让美股盘后/周末全部标的判陈旧、三铁律全天失效 |
| 5 | `risk_events` 带 FK + 表达式唯一索引 | 普通索引 + **应用层去重状态机** | 避免 SQLite 表达式索引兼容性风险；状态机需处理 severity 升级重开、自动解除、确认后仍命中，SQL 唯一索引表达不了 |
| 6 | `external_cash_flows` 6 列 | 增加 `amount_base` / `counted_in_base` | 前者按当时汇率冻结折算额（避免用当前汇率回溯）；后者让 HWM 校准后不重复扣减历史现金流 |
| 7 | 8 张表 / P0 只用 4 张 | 8 张表一次建齐 | 全部 `CREATE TABLE IF NOT EXISTS`，零成本，避免 P1/P2 再来改迁移 |

### 16.4 验证结果（真实数据库回放，2026-11-07）

| 项目 | 结果 |
|---|---|
| `cargo check` | 通过，本次新增代码零警告 |
| `npm run build` | 通过（4776 modules） |
| 实时上下文 | 现金 −329.60 USD、总资产 5286.15 USD、13 个非现金持仓，行情均为 24h 内 |
| 三铁律 | 止损命中 RDDT −36.31% / DDL −32.45% / MOMO −23.08% / GOOG −21.24%；集中度命中 PDD 23.80%（阈值 15%）。价格均为今日 17:03 UTC 刷新，属真实浮亏 |
| HTT 回归 | `holdings.shares=0` → 不进入任何判定；不会再出现「清仓后 +366 虚假盈利」 |
| RDDT 回归 | 有行情按实时价、无行情用 `avg_cost` 兜底 → 不会把 300 买入额显示成 −300 已实现亏损 |
| STALE_DATA 降级 | 行情缺失/超时的标的跳过止损与集中度，仅产生 1 条 info，不用脏价触发斩仓 |
| 出入金免疫 | 入金 +1000 → u=1.000000（不变）；再出金 −300 → u=1.000000（不变）；入金 1000 同时亏损 6% → u=0.94、回撤 −6.00%，正确触发 −5% 熔断 |
| `cargo test` | **受阻**：既有测试代码与生产结构体脱节，17 个编译错误全在 `db/tests.rs` 与 `quote_service.rs` 的历史测试（缺 `pe_ttm`/`f167`/`us_adjust_sell_pay_cost` 等字段），与本次改动无关。本次未混入无关修复，改用上表离线回放验证 |

### 16.5 已知限制与下一步

- **HWM 历史**：熔断保护自启用日起生效，历史回撤请用 `/performance` 的 `DrawdownChart` 查看；需要历史熔断回放时，须先统一 `daily_portfolio_values` 的现金口径。
- **P1 待做**：`watchlist` 冷静期 + `precheck_trade` 交易前置拦截 + `trade_discipline` 留痕（`watchlist`/`trade_discipline` 表已建好）。
- **P2–P4**：LIFO 复盘与时光机、AI 辩证顾问、数据质量校验、合约强平距离规则启用、OS 通知。
- **运行提示**：改的是 Rust 后端，需 `npm run tauri dev` 重新编译后生效。

---

## 17. P1 实施记录（2026-11-07）

### 17.1 交付范围

| 层 | 文件 | 内容 |
|---|---|---|
| 模型 | `models/risk.rs` | `WatchItem / WatchItemInput / TradeVerdict / TradeRiskContext / DisciplineRecord / DisciplineStats` 与 `VERDICT_*`、`RULE_COOLING_PERIOD / RULE_UNPLANNED_TRADE` 常量 |
| 服务 | `services/discipline_service.rs` | 观察列表 CRUD、决策矩阵、留痕、promote、遵守率统计，**8 个单测** |
| 命令 | `commands/discipline.rs` | `get_watchlist / add_watchlist_item / remove_watchlist_item / precheck_trade / get_discipline_stats`，已注册 |
| 交易接入 | `commands/transactions.rs` | `create_transaction` 新增可选 `risk_context`，在同一 `BEGIN IMMEDIATE` 事务内复算判定、拒绝、留痕、promote |
| 前端 | `pages/Risk/index.tsx`、`pages/Transactions/index.tsx`、`stores/riskStore.ts`、`types/index.ts` | 驾驶舱新增「观察列表」Tab（含冷静期遵守率）；交易录入前置校验弹窗（BLOCK 必填理由 / WARN 二次确认） |
| CSV 兼容 | 5 个 Import*Modal | 显式传 `riskContext: null`，历史导入豁免冷静期 |

### 17.2 决策矩阵落地（与 §6.1 一致）

| 场景 | Verdict | 后端处理 |
|---|---|---|
| `SELL` / `PAY` / `OPEN` | PASS | 短路返回，永不拦截 |
| `BUY` 且已有持仓 | PASS | 加仓自由 |
| 观察列表未满 7 天 | BLOCK | 必须填覆盖理由，否则整个事务回滚 |
| 观察列表已满 7 天 | PASS | 记录 `cooling_days` |
| 无持仓且不在观察列表 | WARN | 前端二次确认，`ack_warning=true` 才放行；后端也校验该标志 |
| 组合回撤触及熔断线 | BLOCK | 仅对**新开仓**生效，可把 PASS/WARN 升级为 BLOCK |

规则开关与阈值全部从 `risk_rules` 动态读取（COOLING_PERIOD 7 天、UNPLANNED_TRADE、MAX_DRAWDOWN −5%），UI 可调即生效。

### 17.3 与原设计的偏差（均有依据）

| # | 原设计 | 实施 | 理由 |
|---|---|---|---|
| 1 | 所有 BUY 都走拦截 | 仅**手动录入**走；CSV/图片导入显式豁免 | 5 个导入弹窗回填的是历史成交，用今天的冷静期拦历史数据没有意义，且会直接破坏批量导入 |
| 2 | 后端无条件拦截 | `risk_context: Option<TradeRiskContext>`，`None` 跳过校验、`Some` 时在事务内**独立复算** | 兼容导入路径，同时保证手动录入的 BLOCK 无法被前端绕过（不信任前端传的 verdict） |
| 3 | 熔断期间禁止所有新资金投入 | 熔断只拦**新开仓**（无持仓），已有持仓加仓放行 | 温和策略：避免熔断时连降低平均成本的补仓都被挡住；卖出始终放行 |
| 4 | 覆盖理由仅 BLOCK 必填 | BLOCK 必填理由；WARN 需 `ack_warning` 二次确认 | WARN 不强制写理由，但必须显式确认，两者都落 `trade_discipline` |
| 5 | 观察列表重复加入未定义 | `ON CONFLICT` 保留原 `added_at` | 防止「删了重加 / 重新提交」重置冷静期时钟；已有单测覆盖 |
| 6 | 命令参数由前端传 verdict | 后端在 `create_transaction` 内重算 | `create_transaction` 已持有 `db.conn` 锁并开了事务，所有新函数只接受 `&Connection`，**绝不调用 `build_context(db)`**（std::sync::Mutex 非重入，会死锁） |

### 17.4 验证结果

| 项目 | 结果 |
|---|---|
| `cargo test --lib` | **220 passed / 0 failed / 8 ignored**（含新增 8 个决策矩阵单测） |
| 单测覆盖 | SELL 豁免、加仓放行、非计划 WARN、冷静期 BLOCK 与剩余天数、期满 PASS、熔断升级、规则停用放行、重复加入不重置时钟 |
| `npx tsc --noEmit` | 零错误 |
| `npm run build` | 通过 |
| `check_sql_bindings.py` | 187 处绑定全匹配 |
| 真实库副本回放 | 观察列表写入 → 冷静期 2/7 天判 BLOCK → 同事务留痕 + promote → 遵守率聚合，全链路通过 |

### 17.5 已知限制与下一步

- **留痕范围**：只有手动录入（传 `riskContext`）的交易才写 `trade_discipline`；历史/CSV 导入不产生纪律记录，因此遵守率的分母是「启用 P1 之后的手动交易」，不是全量交易。
- **冷静期可绕过**：用户仍可手动「移除观察项后重新加入」来重置时钟（删除会真正删行）。本地单机 + 温和策略下接受此边界；若需强约束，P2 可改为软删除并保留首次 `added_at`。
- **P2 待做**：LIFO 完整交易配对、时光机 T+30 强制复盘（优先队列取 `override_reason` 非空的记录）、纪律分、投资笔记 + 持仓快照。`investment_memos` / `trade_reviews` 表已建好但仍未接线。
- **运行提示**：改的是 Rust 后端，需 `npm run tauri dev` 重新编译后生效。

---

---

## 18. P2 实施记录（2026-11-07）

### 18.1 交付范围

| 层 | 文件 | 内容 |
|---|---|---|
| 模型 | `models/reflection.rs` | `RoundTrip / RoundTripStats / DueReview / DisciplineScore / HoldingsSnapshot / InvestmentMemo / MemoComparison` |
| 服务 | `services/reflection_service.rs` | 纯函数 `pair_round_trips`（LIFO）+ 7 个单测；时光机补齐、复盘回填、纪律分、笔记快照冻结与对照 |
| 命令 | `commands/reflection.rs` | 10 个命令，`lib.rs` 已注册 |
| 交易接线 | `commands/transactions.rs` | `INSERT transactions` 后在同一 `BEGIN IMMEDIATE` 内 `ensure_time_capsule`（幂等） |
| 前端 | `pages/Risk/index.tsx`、`stores/riskStore.ts`、`types/index.ts` | 新增「复盘」「投资笔记」Tab：纪律分四分项、LIFO 明细、T+30 队列、笔记当时 vs 现在对照 |

### 18.2 LIFO 口径

- 按 `(account_id, UPPER(symbol))` 分组、时间排序；`BUY` / `OPEN` 入栈，`SELL` 从栈顶 LIFO 消耗，`PAY` 不参与配对。
- 一次 SELL 跨多个批次时生成**一条** round-trip（数量加权平均开仓价），用 `split_lots` 标记；`holding_days` 取被消耗的最近批次（LIFO 语义）。
- 手续费按成交股数比例分摊到两腿：`pnl = 卖出净额 − 买入成本 − 两腿手续费`。
- `OPEN` 参与配对但不产生现金流；未平仓批次只统计数量、不计入胜率。

### 18.3 与原设计的偏差

| # | 原设计 | 实施 | 理由 |
|---|---|---|---|
| 1 | 每笔成交都生成时光机 | 仅 `BUY`/`OPEN` 建仓决策生成 | 复盘对象是「当初为什么买」；SELL 无独立 thesis，生成 6 条空笔记只会淹没队列。SELL 的复盘通过对应建仓批次体现 |
| 2 | 复盘按 due_at 到期才出现 | 历史补齐后 18 笔立即进入到期队列 | 历史交易本就该复盘，不隐藏；`days_overdue` 显示逾期天数 |
| 3 | 纪律分四项固定权重 | 分母为 0 的分项不计入，权重在可用项间重归一化，并标 `low_sample` | 新系统上线初期多数分项无样本，按 0 分处理会把纪律分无意义地打到 25 分 |
| 4 | 冷静期分母＝新建仓次数 | 以 `trade_discipline` 中 `is_unplanned=1 OR cooling_days IS NOT NULL` 识别新开仓 | `trade_discipline` 未存 `has_position`，用已有字段还原；加仓为 PASS 且无标记，不进分母 |
| 5 | 笔记快照含 HWM 与风控事件 | 含总资产/现金/单位净值/峰值/持仓权重/活跃事件 | 与 §8.4 一致；快照创建时 JSON 冻结，之后不再重算 |

### 18.4 验证结果（真实库副本回放）

| 项目 | 结果 |
|---|---|
| 24 笔历史成交 | 配对出 5 笔 round-trip：NTES +69 / LEGN +70 / FISV −65 / NOK −11.25 / **HTT −56**；14 个未平仓批次 |
| 聚合指标 | 胜率 40%（2 胜 3 负）、盈亏比 1.0510、总已结盈亏 +6.75、平均持有天数可算 |
| 回归 | HTT 清仓亏损不再被算成 +366 盈利（`c3d4306` 的生产 bug） |
| 时光机 | 补齐 21 条（19 笔历史 BUY/OPEN + 2 笔回放模拟成交），二次执行 0 条，幂等成立 |
| 复盘 | 决策质量回填后完成率 1/18 = 5.6%，与 SQL 聚合一致 |
| 笔记 | 冻结快照 JSON 可解析，含持仓/权重/净值/事件 |
| 单测 | 7 个纯函数单测（LIFO 顺序、部分卖出、跨批次、手续费、分红豁免、分组隔离、空输入）全通过 |

---

## 19. P3 实施记录（2026-11-07）

### 19.1 交付范围

| 层 | 文件 | 内容 |
|---|---|---|
| 模型 | `models/advisor.rs` | `Citation / Challenge / TradeAnalysis / AdvisorReply / LocalEvidence` |
| 服务 | `services/advisor_service.rs` | 本地 6 源检索（中英混合分词 + 词频评分）、OpenAI 兼容 LLM 调用、结构化 JSON 解析、引用反查、无 Key 降级、结果落 `trade_reviews('ai_analysis')` |
| 安全 | `services/secret_store.rs`、`services/ai_config_service.rs` | AES-256-GCM 加密 `api_key`、前端掩码回传、掩码回显不覆盖原值、兼容旧明文 |
| 命令 | `commands/advisor.rs` | `chat_with_advisor` / `analyze_trade`（async） |
| 前端 | 风控中心「辩证顾问」Tab、AI 设置页提示 | 质询输入、单笔成交下拉、观点与引用渲染、降级标识 |

### 19.2 关键约束的落地方式

- **不给买卖建议**：硬编码系统提示词首条即禁止买入/卖出/目标价/仓位指令；用户自定义 `system_prompt` 追加为「可引用背景」，明确不得覆盖硬规则。
- **有据可查**：模型返回的每条 `citation` 都与本轮检索到的 `(source, ref_id)` 逐一比对；未匹配的保留观点但把引用置空并标注「未匹配到本地证据」，不编造出处。
- **数据不出本机**：检索源为 `investment_memos / transactions.notes / risk_events / trade_reviews / watchlist.thesis / risk_rules`；只有拼好的问题文本发往用户自己配置的 LLM 端点。
- **无 Key 可用**：`ai_config` 无行或 key 为空时返回本地证据清单（`degraded=true`），不抛错、不阻塞 UI。
- **可追溯**：`analyze_trade` 成功后 upsert 一条 `ai_analysis` 复盘记录（原文 + 引用 JSON），同一交易重复分析不产生重复行。

### 19.3 与原设计的偏差

| # | 原设计 | 实施 | 理由 |
|---|---|---|---|
| 1 | SQLite FTS5 全文检索 | 关键词词频评分（中文双字 gram + 英文词） | 个人笔记量级小（当前 0–几十条），FTS5 需建虚拟表 + 触发器同步，收益不抵复杂度；量级上千后再升级 |
| 2 | `ai::chat(message, conversation_id)` 多轮 | 单轮质询 | 定位是「照镜子」不是聊天机器人；多轮上下文会放大附和倾向，与辩证目标冲突 |
| 3 | API Key 明文（P4 再加密） | P3 即接入 AES-256-GCM | 加密层与读取层本就同一处，拆到 P4 会让 P3 期间产生一段明文写入窗口 |
| 4 | `user_profile` 表 | 复用 `ai_config.system_prompt` | 首版避免过度设计，与 §9.2 的「首版可先用」一致 |

### 19.4 加密的诚实边界

`enc:v1:<base64(nonce ‖ AES-GCM 密文)>`，密钥由 `SHA-256(应用盐 ‖ USERPROFILE ‖ USERNAME)` 派生，**不落数据库**。防的是「数据库文件被复制/备份/同步后泄露凭据」。同机同用户运行的进程仍能派生同一密钥——要防这个必须上 OS keychain / DPAPI 交互授权，会破坏跨平台构建并引入弹窗，本版明确不做。旧明文值无前缀、可直接读取，下次保存时自动转为密文。

---

## 20. P4 实施记录（2026-11-07）

### 20.1 交付范围

| 层 | 文件 | 内容 |
|---|---|---|
| 校验 | `services/data_quality.rs` | 纯函数 `validate_price / validate_rate / validate_unit_nav` + 5 个单测；只读体检 `check()` 扫价格范围/跳变/大小写重复/陈旧/汇率/净值跳变 |
| 写入防护 | `services/quote_service.rs` | `save_quotes_to_db` 写入前用上一缓存价校验，脏价跳过并输出日志；`risk_service::update_nav_state` 统一调用 `validate_unit_nav` |
| 压力试算 | `services/stress_service.rs` | `symbol="*"` 全局冲击 + 单标的覆盖（后者优先）；只读构建上下文、不推进 HWM、不落事件；复用 `risk_engine::evaluate_all` |
| 上下文隔离 | `services/risk_service.rs` | 新增 `build_context_with_options(db, persist_nav)`，`persist_nav=false` 仅读 HWM |
| 合约规则 | `services/contract_risk_service.rs` | 强平价近似、强平距离、分级 severity、现价三级兜底；6 个单测 |
| 命令 | `commands/enhancement.rs` | `stress_test / evaluate_contract_risk / check_data_quality` |
| 迁移修复 | `db/mod.rs` | 一次性修正早期库 `STALE_DATA=60min` 漂移为 24h，幂等且尊重用户后续自定义 |
| 前端 | 风控中心「压力测试」Tab | 全局/单标的冲击试算、合约强平距离表、数据质量体检报告 |

### 20.2 关键修复：STALE_DATA 配置漂移

`INSERT OR IGNORE` 不会更新已存在行，导致真实库停留在首版播种的 **60 分钟**，而代码默认已是 1440 分钟。后果是美股盘后与周末所有标的被判行情陈旧，`STOP_LOSS / CONCENTRATION` 全部跳过判定、三铁律静默失效。迁移条件精确匹配 `threshold=60.0 AND description='行情超过 N 分钟未更新'`，执行时同步改写 description，因此幂等；用户之后手动调回 60 也不会被覆盖（回放脚本已断言）。

### 20.3 与原设计的偏差

| # | 原设计 | 实施 | 理由 |
|---|---|---|---|
| 1 | 合约规则「P4 启用」 | 默认仍停用，但命令始终返回全部仓位的距离预览 | 规则停用时 UI 已有信息价值；启用与否只影响 severity 是否升级，不改变用户交易行为，避免上线即新增告警噪声 |
| 2 | 压力测试写 `RiskDashboard` | 返回独立 `StressResult`（含前后对比与 deltas） | 驾驶舱是真实状态，不能被假设场景污染；独立结构也便于并排展示 |
| 3 | OS 系统通知 | **不做** | 需引入 `tauri-plugin-notification` + 权限配置，且 §10.2 已定「行情陈旧用灰色角标、不用弹窗打扰」；与用户极简偏好冲突，列入后续可选 |
| 4 | 数据校验拒绝时记 `STALE_DATA` 事件 | 写日志跳过 + 独立体检报告 | 避免脏数据本身制造大量风控事件，反而淹没真实预警 |
| 5 | 强平价按交易所公式 | 逐仓近似（维持保证金率 0.5%） | 交易所分档维持保证金、全仓/逐仓、资金费率差异大；作为「距离预警信号」足够，明确标注为估算 |

### 20.4 验证结果

| 项目 | 结果 |
|---|---|
| `cargo test --lib` | **253 passed / 0 failed / 8 ignored**（本次新增 33 个单测） |
| `npx tsc --noEmit` | 零错误 |
| `npm run build` | 通过 |
| `check_sql_bindings.py` | 199 处绑定匹配、2 处动态 SQL 跳过 |
| `tests/replay_risk_pipeline.py` | **43/43 通过**（P0 基线 / P1 事务级拦截 / P2 LIFO 真实回放 / P4 迁移与合约） |
| 数据质量体检 | 真实库识别到 `TCOM` / `tcom` 大小写重复行情行 |
| 迁移 | 60 → 1440 分钟修正成功，二次执行不覆盖用户自定义 |

---

*§1–§15 为系统设计；§16–§20 依次记录 P0–P4 的实际落地情况与偏差。问题清单与优化建议见 `docs/risk-system-review.md`。文档随实现同步更新。*
