# 风控系统 · 自测结论 / 问题清单 / 优化建议

> 项目：stock-portfolio-manager · 分支 `feat/crypto-spot` · 2026-11-07
> 范围：P1 端到端自测 + P2–P4 实施后的全量复核

---

## 0. 结论先行

**P2–P4 已全部落地，自测全绿，可以进入真实点验。** 本轮新增 33 个 Rust 单测（累计 253 通过）、43 项真实库副本回放、199 处 SQL 绑定检查，前端类型检查与构建均通过。

但**「编译通过」不等于「可放心依赖数值」**。复核发现 4 个会影响判断正确性的问题（A 级），其中最重要的是：**加密合约缺实时行情，强平距离监控目前形同虚设**；**LIFO 盈亏未按币种折算**，一旦有 HK/CN 持仓，胜率与总盈亏会混币种相加。这两项建议在下轮优先处理，本次未做是因为都需要改动既有行情链路，属于独立变更。

另外发现并修复了一个**真实生产配置漂移**：`STALE_DATA` 阈值在真实库里停留在 60 分钟（代码默认已是 24 小时），意味着美股盘后与周末三铁律会静默失效。已加幂等迁移。

---

## 1. 自测结果

| 检查项 | 命令 | 结果 |
|---|---|---|
| Rust 单元/集成测试 | `cargo test --lib` | **253 passed / 0 failed / 8 ignored**（本轮 +33） |
| Rust 编译 | `cargo check` | 通过，16 个既有警告，无新增错误 |
| 前端类型 | `npx tsc --noEmit` | 零错误 |
| 前端构建 | `npm run build` | 通过（4.33 MB bundle，见 C3） |
| SQL 绑定防呆 | `python tests/check_sql_bindings.py` | 199 处匹配，2 处动态 SQL 跳过 |
| 真实库副本回放 | `python tests/replay_risk_pipeline.py` | **43/43 通过** |

### 回放覆盖的关键断言

| 阶段 | 断言 |
|---|---|
| P0 | 8 张表存在、7 条规则已播种、HWM 已锚定 |
| P1 | 重复加入不重置冷静期时钟；2/7 天判 BLOCK；**BLOCK 无理由整笔回滚**；填理由后成交+留痕+promote 同事务；SELL 不拦截；CSV 导入豁免；遵守率聚合 |
| P2 | 24 笔历史成交 LIFO 配对出 5 笔（NTES +69 / LEGN +70 / FISV −65 / NOK −11.25 / **HTT −56**），14 个未平仓批次，胜率 40%、盈亏比 1.0510；时光机补齐 21 条且幂等；复盘回填与完成率；笔记快照冻结 |
| P4 | 识别 `TCOM`/`tcom` 重复行情；STALE_DATA 60→1440 迁移成功且**用户后续自定义不被覆盖**；5x 空单强平价高于开仓价；1x 类现货仓位识别；全局冲击单调性 |

---

## 2. 本轮已修复的问题

| # | 问题 | 根因 | 处理 |
|---|---|---|---|
| F1 | **STALE_DATA 阈值漂移导致三铁律静默失效** | 首版播种 60 分钟，后改为 1440；`INSERT OR IGNORE` 不更新已有行 | 迁移精确匹配 `threshold=60 AND 旧 description`，执行时改写 description 保证幂等；回放断言用户改回 60 不会被覆盖 |
| F2 | **API Key 明文存 SQLite，且明文回传前端** | `ai_config_service` 直接读写原文 | AES-256-GCM + 账户绑定密钥（密钥不落库）、`get_ai_config` 掩码回传、掩码回显不覆盖原值、兼容旧明文渐进迁移 |
| F3 | **脏行情可触发假熔断** | 行情写入无校验 | `save_quotes_to_db` 写入前校验范围与 >95% 跳变；`update_nav_state` 改用统一的 `validate_unit_nav` |
| F4 | **压力测试会污染真实 HWM** | 原 `build_context` 无条件 `update_nav_state` | 拆出 `build_context_with_options(db, persist_nav)`，试算走 `false` |
| F5 | **P2 服务二次加锁会死锁** | `Database` 是非重入 Mutex | 所有新 DB 函数只接受 `&Connection`；AI 服务在已持有 conn 时用 `read_raw_config(&conn)` |
| F6 | **P1 复盘与笔记表建好但未接线** | P0 预留表 | 时光机同事务写入、笔记快照冻结、纪律分聚合、复盘回填全部接通 |

---

## 3. 待处理问题（按严重度）

### A 级 · 会影响判断正确性，建议下轮优先

| # | 问题 | 影响 | 根因 / 证据 | 建议动作 |
|---|---|---|---|---|
| A1 | **加密合约缺实时行情，强平距离恒定** | `ETH 5x short`、`BNB 5x long` 无缓存报价时回落 `open_price`，距离永远不变，规则即使启用也监控不到风险 | `crypto_contract` 9 行中加密标的 2 个；`cached_quotes` 22 行全是 TradFi 与 `$CASH-USD`，无 ETH/BNB | 前端已有 Gate.io 行情轮询，把 `quotes[symbol].price` 组装成 `ContractPriceInput[]` 传给 `evaluate_contract_risk`；后端三级兜底已就绪，只差调用方接线 |
| A2 | **LIFO 盈亏未按币种折算** | 目前 24 笔全是 USD 所以正确；一旦有 HK/CN 持仓，`total_realized_pnl` 会把 USD/HKD/CNY 直接相加，胜率与盈亏比失真 | `transactions.currency` 有值但 `RoundTrip` 未带币种；`pair_round_trips` 用原币 price | `TxRow` 增加 `currency`，配对时按成交日汇率（或 `cached_exchange_rates`）折算到 USD，并新增 `pnl_base` 字段；聚合只用 base 口径 |
| A3 | **>95% 价格跳变拒绝会自锁** | 拆股/合股/代码变更后，旧缓存价永远不更新，新价永远被拒，行情永久停在旧值 | `validate_price(old, new)` 只看上一缓存价，没有「人工确认强制刷新」通道 | 增加 `force_refresh` 参数或设置页「清除该标的缓存」；或允许连续 N 次同向跳变后放行并记一条数据质量事件 |
| A4 | **行情陈旧时三铁律静默降级，状态仍显示「正常」** | 行情源被网关拦截（雪球/东财）时，止损与集中度全部跳过判定，驾驶舱只多 1 条 info，容易误以为风控在正常工作 | `risk_engine::quote_usable` 跳过 → `findings` 只剩 STALE_DATA(info) → `status=normal` | `get_dashboard` 在 `missing_symbols > 0` 时把状态显式改为 `degraded`（新枚举），并在三张铁律卡上打灰角标「未参与判定」 |

### B 级 · 口径与体验

| # | 问题 | 影响 / 建议 |
|---|---|---|
| B1 | 冷静期可通过「移除观察项→重新加入」重置 `added_at` | 本地单机温和策略下接受；若需强约束改软删除并保留首次 `added_at`（§17.5 已记录） |
| B2 | 纪律分分母只含手动录入 | CSV/图片导入豁免 `trade_discipline`，分母偏小；已用 `low_sample` 标注，但不代表全量纪律表现 |
| B3 | 时光机只覆盖 BUY/OPEN | SELL 无独立复盘队列；设计 §8.2 原文是「每笔成交」，属有意偏差（§18.3） |
| B4 | `PAY` 分红调整 `avg_cost`，LIFO 仍用原始买入价 | 净成本法与 LIFO 成本口径不一致，单标的已实现盈亏与持仓成本可能对不上；需定义是否把分红回冲进 round-trip |
| B5 | AI 引用只与本轮 top-8 证据比对 | 模型引用了第 9 条以后的笔记会被标「未匹配」，属误报；可放宽到 top-20 或按 `(source, ref_id)` 回库核验 |
| B6 | 本地检索是词频而非语义 | 中文双字 gram 对同义表述无召回；笔记量级上千后建议升级 SQLite FTS5 |
| B7 | 合约强平价是逐仓近似（维持保证金率 0.5%） | 未含分档维持保证金、全仓/逐仓、资金费率；只能作距离预警，不能当交易所强平价（UI 已标注「预估」） |
| B8 | 真实库 `ai_config` 0 行，AI 顾问默认降级 | 需到 `/ai` 页配置 Key 与模型；用户环境存在代理/DNS 劫持，建议用自定义 `base_url` 指向可用端点 |

### C 级 · 工程与性能

| # | 问题 | 影响 / 建议 |
|---|---|---|
| C1 | `build_context` 每次调用都 `UPDATE portfolio_hwm` | 驾驶舱 30s 轮询 → 每 30 秒一次无变化写库；改为仅 `peak/current` 变化时写（本轮已让压力测试隔离，常规路径未改） |
| C2 | 2 处动态 SQL 被绑定检查跳过 | `list_due_reviews` / `list_memos` 用 `format!` 拼常量片段；改成两条静态 SQL 可让防呆覆盖 100% |
| C3 | 前端单 bundle 4.33 MB（gzip 1.43 MB） | Vite 已告警；按路由 `React.lazy` + `manualChunks` 拆分，尤其 `/risk` 新页体积较大 |
| C4 | 16 个既有 Rust 警告 | 未用导入（`tauri::State`、`ContractPerformanceSummary`、`Datelike`）、未用常量（`STATUS_ACTIVE` 等）、`crypto_quotes` 字段名非 snake_case；零风险清理 |
| C5 | `Database` 是非重入 `Mutex<Connection>` | P1/P2/P3 三次踩到「函数内二次加锁会死锁」，目前靠「所有 DAO 只接 `&Connection`」的约定；可换 `parking_lot::RwLock` 或抽出统一 DAO 层消除隐性约束 |
| C6 | `cached_quotes` 存在 `TCOM`(44.425) / `tcom`(48.07) 重复行 | `build_context` 按 `updated_at` 取最新规避，但两行长期共存且价格不一致；体检已能报告，清理需先确认是否有历史引用 |
| C7 | 无 UI 层自动化回归 | 本轮验证靠 Rust 单测 + Python 数据层回放；`precheck_trade` 弹窗、BLOCK 覆盖理由等交互仍需手动点验 |

---

## 4. 系统可优化点（按投入产出排序）

| 优先级 | 优化 | 预期收益 | 成本 | 排除理由（若本轮未做） |
|---|---|---|---|---|
| 1 | 前端传入 Gate.io 实时价给合约风控 | A1 直接解决，9 个合约仓位真正受监控 | 低（后端接口已就绪） | 需改 `CryptoContract` 页与风控页的数据桥接，属独立变更 |
| 2 | 行情「强制刷新 / 清缓存」通道 | 解除 A3 自锁 | 低 | 需要新增 UI 入口与确认交互，本轮聚焦后端 |
| 3 | LIFO 折算到基准货币 | A2 直接解决，跨市场指标可信 | 中（需历史汇率口径） | 当前真实库全 USD，改动收益暂不显现，但应在引入 HK/CN 交易**之前**完成 |
| 4 | 驾驶舱新增 `degraded` 状态 | A4 直接解决，避免「静默失效」 | 低 | 涉及状态枚举与前端样式，本轮未纳入 |
| 5 | `build_context` 读写分离 | 消除 30s 轮询写放大 | 低 | — |
| 6 | 路由级 code split | 首屏体积显著下降 | 中 | 全站改造，建议单独一轮 |
| 7 | 静态化动态 SQL + 清理 16 个警告 | 防呆覆盖 100%、编译输出干净 | 低 | 纯技术债，可随手清理 |
| 8 | 冷静期软删除 | 堵住 B1 绕过 | 低 | 需新增 `deleted_at` 迁移与查询条件 |
| 9 | FTS5 全文检索 / 引用回库核验 | 提升 AI 召回与引用准确率 | 中 | 当前笔记量级为 0，过早优化 |
| 10 | 汇率 / 波动率冲击、多情景对比 | 压力测试更完整 | 中 | 首版先验证价格冲击链路 |
| 11 | OS 通知 | 离线也能收到 Block 级熔断 | 中 | **明确不做**：需 `tauri-plugin-notification` + 权限配置；§10.2 已定「灰色角标、不弹窗打扰」，与极简偏好冲突 |

### 明确的排除项（含理由）

- **不做 DPAPI / OS keychain**：会破坏跨平台构建并引入交互授权；当前 AES-GCM 已覆盖「数据库文件外泄」这一实际威胁（§19.4 写明边界）。
- **不做多轮 AI 对话**：辩证顾问的职责是质疑，多轮上下文会放大附和倾向（§19.3）。
- **不做云同步 / 云 RAG / 邮件推送**：沿用 §2.3 既有决策，本地单机是前提。
- **不做暗黑 Bloomberg 风**：沿用 light + Ant Design，与全站一致。

---

## 5. 下轮建议的验收动作（需重启 `npm run tauri dev`）

1. 风控中心 →「复盘」点「补齐历史复盘」，确认 19 笔历史建仓进入到期队列、破例记录排在最前。
2. 新建一笔 BUY：观察期未满 → BLOCK 必填理由；理由写入后同事务出现 `trade_discipline` 记录，且 `trade_reviews` 自动生成 T+30 待办。
3. 写一条投资笔记 →「对照现在」，确认冻结快照与当前持仓并排展示。
4. 到 `/ai` 配置 Key →「辩证顾问」发起质询，确认返回带引用、且不含买卖指令；未配 Key 时确认降级不报错。
5. 「压力测试」设全局 −10% + 单标的 −30%，确认回撤变化、findings 与「前后 HWM 未变」。
6. 点「合约强平距离」，确认加密标的当前标记为「缺少实时报价」（即 A1 的现场证据）。

---

## 6. 证据与出处

**验证命令与输出**（2026-11-07，工作区 `F:/WorkSpace/stock-portfolio-manager`）

| 来源 | 核实内容 |
|---|---|
| `cargo test --lib` | 253 passed / 0 failed / 8 ignored；新增 `reflection_service` 7、`advisor_service` 6、`secret_store` 4、`data_quality` 5、`stress_service` 2、`contract_risk_service` 6、`ai_config_service` 2 等单测 |
| `tests/replay_risk_pipeline.py` | 43/43；真实库副本 `data/com.portfolio.manager/portfolio.db`，不污染生产数据 |
| `tests/check_sql_bindings.py` | 199 处 `params!` 与 SQL 占位符匹配，2 处动态 SQL 跳过 |
| `npx tsc --noEmit` / `npm run build` | 零类型错误；构建通过，bundle 4.33 MB |
| 真实库 `risk_rules` | 修复前 `STALE_DATA=60.0 / '行情超过 N 分钟未更新'`；应用迁移后 `1440.0 / 新说明` |
| 真实库 `cached_quotes` | 22 行，含 `TCOM 44.425` 与 `tcom 48.07` 大小写重复；无 ETH/BNB |
| 真实库 `crypto_contract` | 9 行，4 个 1x 类现货、2 个加密标的（ETH 5x short / BNB 5x long） |
| 真实库 `transactions` | 24 行（12 OPEN / 7 BUY / 5 SELL），全部 USD；LIFO 配对期望值见 §1 |
| 设计文档 | `docs/risk-control-system-design.md` v1.3 §16–§20（P0–P4 实施记录与偏差） |
| 本次新增代码 | `models/{reflection,advisor,enhancement}.rs`、`services/{reflection_service,advisor_service,secret_store,data_quality,stress_service,contract_risk_service}.rs`、`commands/{reflection,advisor,enhancement}.rs`、`pages/Risk/index.tsx`、`stores/riskStore.ts`、`types/index.ts`、`db/mod.rs`、`services/{risk_service,quote_service,ai_config_service}.rs` |
