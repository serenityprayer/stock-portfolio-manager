import { useEffect, useState } from "react";
import {
  Alert,
  Button,
  Card,
  DatePicker,
  Descriptions,
  Empty,
  Input,
  InputNumber,
  Modal,
  Popconfirm,
  Progress,
  Select,
  Spin,
  Switch,
  Table,
  Tabs,
  Tag,
  Tooltip,
  Typography,
  message,
} from "antd";
import { ReloadOutlined } from "@ant-design/icons";
import dayjs from "dayjs";
import { invoke } from "@tauri-apps/api/core";
import { useRiskStore } from "../../stores/riskStore";
import type {
  DueReview,
  ExternalCashFlow,
  InvestmentMemo,
  RiskEvent,
  RiskRuleConfig,
  RiskSeverity,
  RoundTrip,
  StressOverride,
  WatchItem,
} from "../../types";

const { Title, Text } = Typography;
const RED = "#E53935";
const GREEN = "#43A047";

const SEVERITY: Record<RiskSeverity, { color: string; label: string }> = {
  block: { color: "red", label: "熔断" },
  critical: { color: "volcano", label: "高危" },
  warning: { color: "orange", label: "预警" },
  info: { color: "default", label: "提示" },
};

const STATUS: Record<string, { color: string; label: string }> = {
  normal: { color: "success", label: "正常" },
  warning: { color: "warning", label: "预警" },
  blocked: { color: "error", label: "熔断" },
};

const FLOW_LABEL: Record<string, string> = {
  DEPOSIT: "入金",
  WITHDRAW: "出金",
  DIVIDEND_ADJUST: "资产校准",
};

function pctColor(v: number) {
  if (Math.abs(v) < 0.005) return "#595959";
  return v >= 0 ? RED : GREEN;
}

function fmtTime(s: string) {
  if (!s) return "-";
  return s.replace("T", " ").slice(0, 16).replace("Z", "");
}

function MetricCard({
  label,
  value,
  valueColor,
  hint,
  percent,
}: {
  label: string;
  value: string;
  valueColor?: string;
  hint: string;
  percent: number;
}) {
  return (
    <Card size="small" styles={{ body: { padding: "16px 18px" } }}>
      <Text type="secondary" style={{ fontSize: 12 }}>{label}</Text>
      <div style={{ fontSize: 26, fontWeight: 600, margin: "6px 0 2px", color: valueColor }}>
        {value}
      </div>
      <Text type="secondary" style={{ fontSize: 12 }}>{hint}</Text>
      <Progress
        percent={Math.min(100, Math.max(0, percent))}
        showInfo={false}
        size="small"
        strokeColor={percent >= 100 ? RED : "#FA8C16"}
        trailColor="#F0F0F0"
        style={{ marginTop: 8, marginBottom: 0 }}
      />
    </Card>
  );
}

export default function RiskPage() {
  const {
    dashboard,
    flows,
    hwm,
    loading,
    error,
    fetchDashboard,
    evaluate,
    acknowledge,
    updateRule,
    fetchFlows,
    fetchHwm,
    recordFlow,
    deleteFlow,
    recalibrate,
    watchlist,
    stats,
    fetchWatchlist,
    addWatchItem,
    removeWatchItem,
    fetchStats,
    roundTrips,
    tripStats,
    dueReviews,
    disciplineScore,
    memos,
    memoComparison,
    analysis,
    advisorReply,
    aiBusy,
    stressResult,
    contractRisks,
    quality,
    fetchRoundTrips,
    fetchDueReviews,
    backfillCapsules,
    completeReview,
    fetchDisciplineScore,
    fetchMemos,
    createMemo,
    deleteMemo,
    compareMemo,
    analyzeTrade,
    askAdvisor,
    stressTest,
    fetchContractRisk,
    fetchQuality,
  } = useRiskStore();

  const [accounts, setAccounts] = useState<Array<{ id: string; name: string }>>([]);
  const [draft, setDraft] = useState<Record<string, number | null>>({});
  const [calOpen, setCalOpen] = useState(false);
  const [calReason, setCalReason] = useState("");
  const [watchForm, setWatchForm] = useState({
    symbol: "",
    name: "",
    market: "US",
    thesis: "",
    targetPrice: null as number | null,
    addedAt: dayjs().format("YYYY-MM-DD"),
  });
  const [form, setForm] = useState({
    accountId: "",
    currency: "USD",
    flowType: "DEPOSIT" as ExternalCashFlow["flow_type"],
    amount: 0,
    occurredAt: dayjs().format("YYYY-MM-DD"),
    note: "",
  });
  const [reviewTarget, setReviewTarget] = useState<DueReview | null>(null);
  const [reviewQuality, setReviewQuality] = useState("correct");
  const [reviewContent, setReviewContent] = useState("");
  const [memoOpen, setMemoOpen] = useState(false);
  const [memoForm, setMemoForm] = useState({ title: "", content: "", tags: "" });
  const [stressGlobal, setStressGlobal] = useState<number>(-10);
  const [stressSymbol, setStressSymbol] = useState("");
  const [stressSymbolPct, setStressSymbolPct] = useState<number>(-30);
  const [aiMessage, setAiMessage] = useState("");
  const [aiTxId, setAiTxId] = useState("");

  useEffect(() => {
    fetchDashboard();
    fetchFlows();
    fetchHwm();
    fetchWatchlist();
    fetchStats();
    fetchRoundTrips();
    fetchDueReviews();
    fetchDisciplineScore();
    fetchMemos();
    invoke<Array<{ id: string; name: string }>>("get_accounts")
      .then((list) => {
        setAccounts(list);
        if (list.length > 0) setForm((f) => ({ ...f, accountId: list[0].id }));
      })
      .catch(() => undefined);
    const timer = window.setInterval(fetchDashboard, 30_000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!dashboard) {
    return (
      <div style={{ padding: 48, textAlign: "center" }}>
        <Spin tip="正在体检…" />
        {error && <div style={{ marginTop: 16, color: RED }}>{error}</div>}
      </div>
    );
  }

  const d = dashboard;
  const fresh = d.quote_freshness;
  const badQuotes = fresh.missing_symbols.length + fresh.stale_symbols.length;
  const ddRatio = d.drawdown_threshold !== 0
    ? Math.abs(d.current_drawdown / d.drawdown_threshold) * 100
    : 0;
  const concentrationRatio = d.concentration_threshold !== 0
    ? (d.max_weight / d.concentration_threshold) * 100
    : 0;

  const eventColumns = [
    {
      title: "级别",
      dataIndex: "severity",
      key: "severity",
      width: 76,
      render: (s: RiskSeverity) => <Tag color={SEVERITY[s]?.color}>{SEVERITY[s]?.label ?? s}</Tag>,
    },
    {
      title: "规则",
      dataIndex: "rule_id",
      key: "rule_id",
      width: 170,
      render: (id: string) => d.rules.find((r) => r.id === id)?.name ?? id,
    },
    { title: "标的", dataIndex: "symbol", key: "symbol", width: 100, render: (v: string | null) => v ?? "组合" },
    { title: "说明", dataIndex: "message", key: "message" },
    {
      title: "最近触发",
      dataIndex: "last_triggered_at",
      key: "last_triggered_at",
      width: 140,
      render: (v: string) => <Text type="secondary" style={{ fontSize: 12 }}>{fmtTime(v)}</Text>,
    },
    {
      title: "",
      key: "action",
      width: 90,
      render: (_: unknown, r: RiskEvent) =>
        r.status === "active" ? (
          <Button size="small" type="link" onClick={() => acknowledge(r.id)}>确认</Button>
        ) : (
          <Text type="secondary" style={{ fontSize: 12 }}>已确认</Text>
        ),
    },
  ];

  const ruleColumns = [
    { title: "规则", dataIndex: "name", key: "name", width: 170 },
    {
      title: "级别",
      dataIndex: "severity",
      key: "severity",
      width: 80,
      render: (s: RiskSeverity) => <Tag color={SEVERITY[s]?.color}>{SEVERITY[s]?.label ?? s}</Tag>,
    },
    {
      title: "启用",
      dataIndex: "enabled",
      key: "enabled",
      width: 76,
      render: (v: boolean, r: RiskRuleConfig) => (
        <Switch size="small" checked={v} onChange={(nv) => updateRule(r.id, nv, null)} />
      ),
    },
    {
      title: "阈值",
      dataIndex: "threshold",
      key: "threshold",
      width: 130,
      render: (v: number | null, r: RiskRuleConfig) =>
        v === null ? (
          <Text type="secondary">—</Text>
        ) : (
          <InputNumber
            size="small"
            style={{ width: 96 }}
            value={draft[r.id] ?? v}
            onChange={(nv) => setDraft((s) => ({ ...s, [r.id]: nv }))}
            onBlur={() => {
              const nv = draft[r.id];
              if (nv !== undefined && nv !== null && nv !== v) updateRule(r.id, null, nv);
            }}
          />
        ),
    },
    { title: "说明", dataIndex: "description", key: "description", render: (v: string | null) => <Text type="secondary" style={{ fontSize: 12 }}>{v}</Text> },
  ];

  const flowColumns = [
    { title: "日期", dataIndex: "occurred_at", key: "occurred_at", width: 110, render: (v: string) => v.slice(0, 10) },
    {
      title: "类型",
      dataIndex: "flow_type",
      key: "flow_type",
      width: 100,
      render: (v: string) => <Tag color={v === "WITHDRAW" ? "green" : "red"}>{FLOW_LABEL[v] ?? v}</Tag>,
    },
    {
      title: "金额",
      dataIndex: "amount",
      key: "amount",
      width: 130,
      render: (v: number, r: ExternalCashFlow) => (
        <span style={{ color: pctColor(v), fontWeight: 500 }}>
          {v >= 0 ? "+" : ""}{v.toFixed(2)} {r.currency}
        </span>
      ),
    },
    {
      title: "账户",
      dataIndex: "account_id",
      key: "account_id",
      width: 150,
      render: (v: string) => accounts.find((a) => a.id === v)?.name ?? v.slice(0, 8),
    },
    { title: "备注", dataIndex: "note", key: "note", render: (v: string | null) => v ?? "-" },
    {
      title: "",
      key: "action",
      width: 70,
      render: (_: unknown, r: ExternalCashFlow) => (
        <Popconfirm title="删除这条出入金？" onConfirm={() => deleteFlow(r.id)} okText="删除" cancelText="取消">
          <Button size="small" type="link" danger>删除</Button>
        </Popconfirm>
      ),
    },
  ];

  const watchColumns = [
    {
      title: "标的",
      key: "symbol",
      width: 140,
      render: (_: unknown, r: WatchItem) => (
        <div>
          <strong>{r.symbol}</strong>
          {r.name && <div style={{ fontSize: 11, color: "#8c8c8c" }}>{r.name}</div>}
        </div>
      ),
    },
    { title: "市场", dataIndex: "market", key: "market", width: 70 },
    {
      title: "加入",
      dataIndex: "added_at",
      key: "added_at",
      width: 100,
      render: (v: string) => v.slice(0, 10),
    },
    {
      title: "已观察",
      key: "cooling",
      width: 90,
      render: (_: unknown, r: WatchItem) => `${r.cooling_days.toFixed(1)} 天`,
    },
    {
      title: "状态",
      key: "eligible",
      width: 130,
      render: (_: unknown, r: WatchItem) =>
        r.eligible ? (
          <Tag color="success">可建仓</Tag>
        ) : (
          <Tag color="orange">还需 {r.days_remaining.toFixed(1)} 天</Tag>
        ),
    },
    {
      title: "逻辑",
      dataIndex: "thesis",
      key: "thesis",
      render: (v: string | null) => v ?? "-",
    },
    {
      title: "",
      key: "action",
      width: 70,
      render: (_: unknown, r: WatchItem) => (
        <Popconfirm
          title="移出观察列表？"
          onConfirm={() => removeWatchItem(r.id)}
          okText="移除"
          cancelText="取消"
        >
          <Button size="small" type="link" danger>移除</Button>
        </Popconfirm>
      ),
    },
  ];

  const tripColumns = [
    { title: "标的", dataIndex: "symbol", key: "symbol", width: 90 },
    { title: "建仓", dataIndex: "open_at", key: "open_at", width: 105, render: (v: string) => v.slice(0, 10) },
    { title: "平仓", dataIndex: "close_at", key: "close_at", width: 105, render: (v: string) => v.slice(0, 10) },
    { title: "持有", key: "holding_days", width: 75, render: (_: unknown, r: RoundTrip) => `${r.holding_days} 天` },
    { title: "数量", dataIndex: "qty", key: "qty", width: 80, render: (v: number) => v.toFixed(2) },
    {
      title: "开/平均价",
      key: "prices",
      width: 140,
      render: (_: unknown, r: RoundTrip) => `${r.avg_open_price.toFixed(2)} → ${r.avg_close_price.toFixed(2)}`,
    },
    {
      title: "已结盈亏",
      key: "pnl",
      width: 120,
      render: (_: unknown, r: RoundTrip) => (
        <span style={{ color: pctColor(r.realized_pnl), fontWeight: 500 }}>
          {r.realized_pnl >= 0 ? "+" : ""}{r.realized_pnl.toFixed(2)} ({r.return_rate >= 0 ? "+" : ""}{r.return_rate.toFixed(1)}%)
        </span>
      ),
    },
    {
      title: "当初理由",
      key: "note",
      render: (_: unknown, r: RoundTrip) => (
        <Tooltip title={r.open_note ?? ""}>
          <Text type="secondary" style={{ fontSize: 12 }}>{r.open_note ?? "—"}</Text>
        </Tooltip>
      ),
    },
    {
      title: "",
      key: "split",
      width: 70,
      render: (_: unknown, r: RoundTrip) => r.split_lots ? <Tag color="blue">跨批次</Tag> : null,
    },
  ];

  const reviewColumns = [
    {
      title: "标的",
      key: "symbol",
      width: 100,
      render: (_: unknown, r: DueReview) => (
        <div>
          <strong>{r.symbol}</strong>
          {r.is_priority && <Tag color="red" style={{ marginLeft: 6 }}>破例</Tag>}
        </div>
      ),
    },
    { title: "成交日", dataIndex: "traded_at", key: "traded_at", width: 105, render: (v: string | null) => v?.slice(0, 10) ?? "—" },
    { title: "到期", dataIndex: "due_at", key: "due_at", width: 105, render: (v: string | null) => v?.slice(0, 10) ?? "—" },
    {
      title: "状态",
      key: "status",
      width: 105,
      render: (_: unknown, r: DueReview) => r.completed_at
        ? <Tag color="success">已复盘 {r.decision_quality === "correct" ? "✓" : r.decision_quality === "wrong" ? "✗" : ""}</Tag>
        : <Tag color={r.days_overdue > 0 ? "orange" : "default"}>{r.days_overdue > 0 ? `逾期 ${r.days_overdue} 天` : "待到期"}</Tag>,
    },
    {
      title: "当初怎么想",
      key: "thesis",
      render: (_: unknown, r: DueReview) => (
        <div style={{ fontSize: 12 }}>
          <div>{r.open_note ?? <Text type="secondary">未记录买入理由</Text>}</div>
          {r.override_reason && <div style={{ color: RED }}>破例理由：{r.override_reason}</div>}
        </div>
      ),
    },
    {
      title: "",
      key: "action",
      width: 90,
      render: (_: unknown, r: DueReview) => (
        <Button
          size="small"
          type="link"
          disabled={!!r.completed_at}
          onClick={() => {
            setReviewTarget(r);
            setReviewQuality("correct");
            setReviewContent(r.content ?? "");
          }}
        >
          {r.completed_at ? "已完成" : "回填复盘"}
        </Button>
      ),
    },
  ];

  const memoColumns = [
    { title: "标题", dataIndex: "title", key: "title", width: 220 },
    { title: "标签", dataIndex: "tags", key: "tags", width: 140, render: (v: string | null) => v ?? "—" },
    { title: "创建", dataIndex: "created_at", key: "created_at", width: 140, render: (v: string) => fmtTime(v) },
    {
      title: "",
      key: "action",
      width: 140,
      render: (_: unknown, r: InvestmentMemo) => (
        <div className="flex gap-1">
          <Button size="small" type="link" onClick={() => compareMemo(r.id)}>对照现在</Button>
          <Popconfirm title="删除这篇笔记？" onConfirm={() => deleteMemo(r.id)} okText="删除" cancelText="取消">
            <Button size="small" type="link" danger>删除</Button>
          </Popconfirm>
        </div>
      ),
    },
  ];

  const stressColumns = [
    { title: "标的", dataIndex: "symbol", key: "symbol", width: 110 },
    { title: "原价", dataIndex: "old_price", key: "old_price", width: 100, render: (v: number) => v.toFixed(2) },
    { title: "假设价", dataIndex: "new_price", key: "new_price", width: 100, render: (v: number) => v.toFixed(2) },
    {
      title: "市值变化",
      key: "delta",
      render: (_: unknown, r: { old_value: number; new_value: number }) => (
        <span style={{ color: pctColor(r.new_value - r.old_value) }}>
          {r.old_value.toFixed(2)} → {r.new_value.toFixed(2)}
        </span>
      ),
    },
  ];

  const contractColumns = [
    { title: "标的", dataIndex: "symbol", key: "symbol", width: 100 },
    { title: "方向", dataIndex: "position_type", key: "position_type", width: 80, render: (v: string) => v === "long" ? "多" : "空" },
    { title: "杠杆", dataIndex: "leverage", key: "leverage", width: 80, render: (v: number) => `${v}x` },
    { title: "开仓价", dataIndex: "open_price", key: "open_price", width: 100, render: (v: number) => v.toFixed(2) },
    { title: "现价", dataIndex: "current_price", key: "current_price", width: 100, render: (v: number) => v > 0 ? v.toFixed(2) : "—" },
    { title: "预估强平价", dataIndex: "liquidation_price", key: "liquidation_price", width: 110, render: (v: number) => v.toFixed(2) },
    {
      title: "强平距离",
      key: "distance",
      width: 110,
      render: (_: unknown, r: { distance_percent: number; severity: RiskSeverity }) => (
        <Tag color={SEVERITY[r.severity]?.color ?? "default"}>{r.distance_percent.toFixed(1)}%</Tag>
      ),
    },
    { title: "说明", dataIndex: "message", key: "message" },
  ];

  // 后端约定 symbol="*" 表示全部非现金持仓；单标的 override 可叠加覆盖。
  const runStress = async () => {
    const overrides: StressOverride[] = [];
    if (stressGlobal) {
      overrides.push({ symbol: "*", price_change_percent: stressGlobal });
    }
    if (stressSymbol.trim()) {
      overrides.push({
        symbol: stressSymbol.trim().toUpperCase(),
        price_change_percent: stressSymbolPct,
      });
    }
    if (!overrides.length) {
      message.warning("请设置至少一个价格冲击");
      return;
    }
    await stressTest(overrides);
  };

  const submitWatch = async () => {
    if (!watchForm.symbol.trim()) {
      message.warning("请填写标的代码");
      return;
    }
    const ok = await addWatchItem({
      symbol: watchForm.symbol.trim().toUpperCase(),
      name: watchForm.name.trim() || null,
      market: watchForm.market,
      currency: watchForm.market === "US" ? "USD" : watchForm.market === "HK" ? "HKD" : "CNY",
      thesis: watchForm.thesis.trim() || null,
      target_price: watchForm.targetPrice,
      added_at: watchForm.addedAt,
    });
    if (ok) {
      message.success("已加入观察列表");
      setWatchForm((f) => ({ ...f, symbol: "", name: "", thesis: "", targetPrice: null }));
    } else {
      message.error(useRiskStore.getState().error ?? "添加失败");
    }
  };

  const submitFlow = async () => {
    if (!form.accountId || form.amount <= 0) {
      message.warning("请选择账户并填写正数金额");
      return;
    }
    const ok = await recordFlow({
      accountId: form.accountId,
      currency: form.currency,
      flowType: form.flowType,
      amount: form.amount,
      occurredAt: form.occurredAt,
      note: form.note || undefined,
    });
    if (ok) {
      message.success("已记录，现金余额与净值已同步");
      setForm((f) => ({ ...f, amount: 0, note: "" }));
    } else {
      message.error(useRiskStore.getState().error ?? "记录失败");
    }
  };

  return (
    <div>
      <div className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-3">
          <Title level={3} className="!mb-0">风控中心</Title>
          <Tag color={STATUS[d.status]?.color}>{STATUS[d.status]?.label ?? d.status}</Tag>
          {badQuotes > 0 && (
            <Tag color="default">
              {badQuotes} 个标的行情缺失/陈旧 · 最旧 {Math.round(fresh.oldest_quote_age_secs / 60)} 分钟
            </Tag>
          )}
        </div>
        <div className="flex items-center gap-3">
          <Text type="secondary" style={{ fontSize: 12 }}>{fmtTime(d.as_of)} UTC</Text>
          <Button icon={<ReloadOutlined />} loading={loading} onClick={evaluate}>立即体检</Button>
        </div>
      </div>

      {d.status === "blocked" && (
        <Alert type="error" showIcon banner message="组合已触发回撤熔断，暂停新开仓" className="!mb-3" />
      )}
      {d.status === "warning" && (
        <Alert type="warning" showIcon banner message="存在需要关注的风控预警" className="!mb-3" />
      )}
      {error && <Alert type="error" showIcon message={error} className="!mb-3" closable />}

      <div className="grid grid-cols-3 gap-4 mb-4">
        <MetricCard
          label="当前回撤"
          value={`${d.current_drawdown.toFixed(2)}%`}
          valueColor={pctColor(d.current_drawdown)}
          hint={`熔断线 ${d.drawdown_threshold.toFixed(0)}% · 峰值净值 ${d.peak_unit_nav.toFixed(4)}`}
          percent={ddRatio}
        />
        <MetricCard
          label="止损持仓"
          value={`${d.stop_loss_count} 个`}
          valueColor={d.stop_loss_count > 0 ? RED : undefined}
          hint={`止损线 ${d.stop_loss_threshold.toFixed(0)}% · 共 ${d.position_count} 个持仓`}
          percent={d.stop_loss_count > 0 ? 100 : 0}
        />
        <MetricCard
          label="最大集中度"
          value={`${d.max_weight.toFixed(2)}%`}
          valueColor={d.max_weight > d.concentration_threshold ? RED : undefined}
          hint={`${d.max_weight_symbol || "—"} · 上限 ${d.concentration_threshold.toFixed(0)}%`}
          percent={concentrationRatio}
        />
      </div>

      <Card size="small" title="待处理预警" className="mb-4">
        <Table
          dataSource={d.events}
          columns={eventColumns}
          rowKey="id"
          size="small"
          pagination={false}
          locale={{ emptyText: "暂无待处理预警" }}
        />
      </Card>

      <Card size="small">
        <Tabs
          size="small"
          items={[
            {
              key: "rules",
              label: "规则设置",
              children: (
                <div>
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    阈值修改后失焦即保存；停用规则后不再参与体检。
                  </Text>
                  <Table
                    dataSource={d.rules}
                    columns={ruleColumns}
                    rowKey="id"
                    size="small"
                    pagination={false}
                    className="mt-2"
                  />
                </div>
              ),
            },
            {
              key: "watchlist",
              label: `观察列表${watchlist.length ? ` (${watchlist.length})` : ""}`,
              children: (
                <div>
                  <div className="flex flex-wrap items-center gap-2 mb-3">
                    <Input
                      size="small"
                      style={{ width: 110 }}
                      placeholder="代码"
                      value={watchForm.symbol}
                      onChange={(e) => setWatchForm((f) => ({ ...f, symbol: e.target.value }))}
                      onPressEnter={submitWatch}
                    />
                    <Input
                      size="small"
                      style={{ width: 130 }}
                      placeholder="名称（选填）"
                      value={watchForm.name}
                      onChange={(e) => setWatchForm((f) => ({ ...f, name: e.target.value }))}
                    />
                    <Select
                      size="small"
                      style={{ width: 85 }}
                      value={watchForm.market}
                      options={[{ value: "US", label: "美股" }, { value: "HK", label: "港股" }, { value: "CN", label: "A股" }]}
                      onChange={(v) => setWatchForm((f) => ({ ...f, market: v }))}
                    />
                    <Input
                      size="small"
                      style={{ width: 190 }}
                      placeholder="买入逻辑（选填）"
                      value={watchForm.thesis}
                      onChange={(e) => setWatchForm((f) => ({ ...f, thesis: e.target.value }))}
                    />
                    <InputNumber
                      size="small"
                      style={{ width: 105 }}
                      placeholder="目标价"
                      min={0}
                      value={watchForm.targetPrice}
                      onChange={(v) => setWatchForm((f) => ({ ...f, targetPrice: v }))}
                    />
                    <DatePicker
                      size="small"
                      value={dayjs(watchForm.addedAt)}
                      allowClear={false}
                      onChange={(v) => setWatchForm((f) => ({ ...f, addedAt: v?.format("YYYY-MM-DD") ?? f.addedAt }))}
                    />
                    <Button size="small" type="primary" onClick={submitWatch}>加入</Button>
                    {stats && stats.total > 0 && (
                      <Text type="secondary" style={{ fontSize: 12, marginLeft: "auto" }}>
                        冷静期遵守率 {stats.compliance_rate.toFixed(0)}% · 覆盖 {stats.overridden} 次 · 非计划 {stats.unplanned} 次
                      </Text>
                    )}
                  </div>
                  <Table
                    dataSource={watchlist}
                    columns={watchColumns}
                    rowKey="id"
                    size="small"
                    pagination={false}
                    locale={{ emptyText: "暂无观察标的" }}
                  />
                </div>
              ),
            },
            {
              key: "cashflow",
              label: "出入金",
              children: (
                <div>
                  <div className="flex flex-wrap items-center gap-2 mb-3">
                    <Select
                      size="small"
                      style={{ width: 150 }}
                      value={form.accountId || undefined}
                      placeholder="账户"
                      options={accounts.map((a) => ({ value: a.id, label: a.name }))}
                      onChange={(v) => setForm((f) => ({ ...f, accountId: v }))}
                    />
                    <Select
                      size="small"
                      style={{ width: 110 }}
                      value={form.flowType}
                      options={[
                        { value: "DEPOSIT", label: "入金" },
                        { value: "WITHDRAW", label: "出金" },
                        { value: "DIVIDEND_ADJUST", label: "资产校准" },
                      ]}
                      onChange={(v) => setForm((f) => ({ ...f, flowType: v }))}
                    />
                    <Select
                      size="small"
                      style={{ width: 90 }}
                      value={form.currency}
                      options={["USD", "CNY", "HKD"].map((c) => ({ value: c, label: c }))}
                      onChange={(v) => setForm((f) => ({ ...f, currency: v }))}
                    />
                    <InputNumber
                      size="small"
                      style={{ width: 130 }}
                      min={0}
                      placeholder="金额"
                      value={form.amount}
                      onChange={(v) => setForm((f) => ({ ...f, amount: v ?? 0 }))}
                    />
                    <DatePicker
                      size="small"
                      value={dayjs(form.occurredAt)}
                      onChange={(v) => setForm((f) => ({ ...f, occurredAt: v?.format("YYYY-MM-DD") ?? f.occurredAt }))}
                      allowClear={false}
                    />
                    <Input
                      size="small"
                      style={{ width: 180 }}
                      placeholder="备注（选填）"
                      value={form.note}
                      onChange={(e) => setForm((f) => ({ ...f, note: e.target.value }))}
                    />
                    <Button size="small" type="primary" onClick={submitFlow}>记录</Button>
                  </div>
                  <Table
                    dataSource={flows}
                    columns={flowColumns}
                    rowKey="id"
                    size="small"
                    pagination={false}
                    locale={{ emptyText: "暂无出入金记录" }}
                  />
                </div>
              ),
            },
            {
              key: "hwm",
              label: "净值基准",
              children: (
                <div>
                  <div className="grid grid-cols-4 gap-4 mb-3">
                    <div><Text type="secondary" style={{ fontSize: 12 }}>单位净值</Text><div style={{ fontSize: 20, fontWeight: 600 }}>{d.unit_nav.toFixed(4)}</div></div>
                    <div><Text type="secondary" style={{ fontSize: 12 }}>历史峰值</Text><div style={{ fontSize: 20, fontWeight: 600 }}>{d.peak_unit_nav.toFixed(4)}</div></div>
                    <div><Text type="secondary" style={{ fontSize: 12 }}>累计净出入金</Text><div style={{ fontSize: 20, fontWeight: 600 }}>{d.net_external_flow >= 0 ? "+" : ""}{d.net_external_flow.toFixed(2)}</div></div>
                    <div><Text type="secondary" style={{ fontSize: 12 }}>总资产 (USD)</Text><div style={{ fontSize: 20, fontWeight: 600 }}>{d.total_assets.toFixed(2)}</div></div>
                  </div>
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {d.hwm_note} 基准日 {fmtTime(hwm?.base_date ?? d.hwm_base_date)}，基准资产 {hwm?.base_value.toFixed(2) ?? "-"} USD；当前现金余额 {d.cash_value.toFixed(2)} USD（负值代表融资）。
                  </Text>
                  <div className="mt-3">
                    <Button size="small" onClick={() => setCalOpen(true)}>校准净值基准</Button>
                  </div>
                </div>
              ),
            },
            {
              key: "review",
              label: `复盘${dueReviews.filter((r) => !r.completed_at).length ? ` (${dueReviews.filter((r) => !r.completed_at).length})` : ""}`,
              children: (
                <div>
                  <div className="flex items-center gap-4 mb-3 flex-wrap">
                    <div>
                      <Text type="secondary" style={{ fontSize: 12 }}>纪律分</Text>
                      <div style={{ fontSize: 28, fontWeight: 600, lineHeight: 1.1 }}>
                        {disciplineScore?.score.toFixed(0) ?? "—"}
                        {disciplineScore?.low_sample && <Text type="secondary" style={{ fontSize: 12, marginLeft: 8 }}>样本偏少</Text>}
                      </div>
                    </div>
                    {disciplineScore && (
                      <Descriptions size="small" column={2} style={{ flex: 1, minWidth: 380 }} items={[
                        { key: "c", label: "冷静期 30%", children: `${disciplineScore.cooling_compliance?.toFixed(0) ?? "—"} · ${disciplineScore.cooling_detail}` },
                        { key: "p", label: "计划交易 25%", children: `${disciplineScore.planned_ratio?.toFixed(0) ?? "—"} · ${disciplineScore.planned_detail}` },
                        { key: "s", label: "止损执行 25%", children: `${disciplineScore.stop_loss_execution?.toFixed(0) ?? "—"} · ${disciplineScore.stop_loss_detail}` },
                        { key: "r", label: "复盘完成 20%", children: `${disciplineScore.review_completion?.toFixed(0) ?? "—"} · ${disciplineScore.review_detail}` },
                      ]} />
                    )}
                    <Button size="small" onClick={async () => {
                      const n = await backfillCapsules();
                      if (n >= 0) message.success(`已补齐 ${n} 笔历史交易的 T+30 复盘`);
                    }}>补齐历史复盘</Button>
                  </div>

                  <div className="grid grid-cols-4 gap-3 mb-3">
                    {[
                      ["已结交易", tripStats?.total_trades ?? 0],
                      ["胜率", `${(tripStats?.win_rate ?? 0).toFixed(0)}%`],
                      ["盈亏比", tripStats?.profit_factor != null ? tripStats.profit_factor.toFixed(2) : "—"],
                      ["已结盈亏", (tripStats?.total_realized_pnl ?? 0).toFixed(2)],
                    ].map(([label, value]) => (
                      <Card key={String(label)} size="small" styles={{ body: { padding: "10px 14px" } }}>
                        <Text type="secondary" style={{ fontSize: 12 }}>{label}</Text>
                        <div style={{ fontSize: 20, fontWeight: 600 }}>{value}</div>
                      </Card>
                    ))}
                  </div>

                  <Text strong style={{ fontSize: 13 }}>T+30 时光机</Text>
                  <Table
                    dataSource={dueReviews}
                    columns={reviewColumns}
                    rowKey="id"
                    size="small"
                    className="mt-2 mb-4"
                    pagination={{ pageSize: 6, size: "small" }}
                    locale={{ emptyText: "暂无复盘待办，点「补齐历史复盘」导入历史成交" }}
                  />

                  <Text strong style={{ fontSize: 13 }}>LIFO 完整交易配对</Text>
                  <Table
                    dataSource={roundTrips}
                    columns={tripColumns}
                    rowKey={(r) => `${r.symbol}-${r.close_at}-${r.open_at}`}
                    size="small"
                    className="mt-2"
                    pagination={{ pageSize: 8, size: "small" }}
                    locale={{ emptyText: "暂无已平仓交易" }}
                  />
                </div>
              ),
            },
            {
              key: "memos",
              label: "投资笔记",
              children: (
                <div>
                  <div className="flex items-center gap-2 mb-3">
                    <Button size="small" type="primary" onClick={() => setMemoOpen(true)}>写笔记</Button>
                    <Text type="secondary" style={{ fontSize: 12 }}>创建时冻结当时的持仓、权重、净值与风控事件，之后不再变动。</Text>
                    <Button size="small" className="ml-auto" onClick={() => compareMemo(null)}>收起对照</Button>
                  </div>
                  <Table
                    dataSource={memos}
                    columns={memoColumns}
                    rowKey="id"
                    size="small"
                    pagination={false}
                    locale={{ emptyText: "暂无笔记" }}
                  />
                  {memoComparison && (
                    <Card size="small" title={`「${memoComparison.memo.title}」· 当时 vs 现在`} className="mt-3">
                      <Descriptions size="small" column={3} items={[
                        { key: "a", label: "当时总资产", children: memoComparison.then_snapshot?.total_assets_base.toFixed(2) ?? "—" },
                        { key: "b", label: "现在总资产", children: memoComparison.now_snapshot.total_assets_base.toFixed(2) },
                        { key: "c", label: "当时单位净值", children: memoComparison.then_snapshot?.unit_nav.toFixed(4) ?? "—" },
                      ]} />
                      <div className="mt-2" style={{ whiteSpace: "pre-wrap", fontSize: 13 }}>{memoComparison.memo.content}</div>
                      <Table
                        className="mt-3"
                        dataSource={memoComparison.changes}
                        rowKey="symbol"
                        size="small"
                        pagination={false}
                        columns={[
                          { title: "标的", dataIndex: "symbol", key: "symbol" },
                          { title: "当时权重", dataIndex: "then_weight", key: "then_weight", render: (v: number) => `${v.toFixed(1)}%` },
                          { title: "现在权重", dataIndex: "now_weight", key: "now_weight", render: (v: number) => `${v.toFixed(1)}%` },
                          { title: "市值变化", key: "change", render: (_: unknown, r: { value_change: number }) => <span style={{ color: pctColor(r.value_change) }}>{r.value_change >= 0 ? "+" : ""}{r.value_change.toFixed(2)}</span> },
                        ]}
                      />
                    </Card>
                  )}
                </div>
              ),
            },
            {
              key: "advisor",
              label: "辩证顾问",
              children: (
                <div>
                  <Text type="secondary" style={{ fontSize: 12, display: "block", marginBottom: 10 }}>
                    只挑战假设、不给买卖指令；观点必须引用本地笔记 / 交易理由 / 风控事件。
                  </Text>
                  <Input.TextArea
                    rows={3}
                    placeholder="例如：我想现在加仓 RDDT，因为已经跌了很多，逻辑有没有问题？"
                    value={aiMessage}
                    onChange={(e) => setAiMessage(e.target.value)}
                  />
                  <div className="flex items-center gap-2 mt-2">
                    <Button size="small" type="primary" loading={aiBusy} onClick={() => askAdvisor(aiMessage)} disabled={!aiMessage.trim()}>发起质询</Button>
                    <Select
                      size="small"
                      style={{ width: 260 }}
                      placeholder="选一笔成交做单笔分析"
                      value={aiTxId || undefined}
                      allowClear
                      options={dueReviews
                        .filter((r) => r.transaction_id)
                        .map((r) => ({
                          value: r.transaction_id as string,
                          label: `${r.symbol} · ${r.traded_at?.slice(0, 10) ?? ""}`,
                        }))}
                      onChange={(v) => setAiTxId(v ?? "")}
                    />
                    <Button size="small" loading={aiBusy} onClick={() => analyzeTrade(aiTxId)} disabled={!aiTxId.trim()}>分析这笔交易</Button>
                  </div>

                  {advisorReply && (
                    <Card size="small" title="质询结果" className="mt-3">
                      <div style={{ whiteSpace: "pre-wrap", fontSize: 13 }}>{advisorReply.reply}</div>
                      {advisorReply.degraded && <Tag color="orange" className="mt-2">未配置 API Key，仅返回本地证据</Tag>}
                      {advisorReply.citations.map((c, i) => (
                        <div key={i} style={{ fontSize: 12, color: "#8c8c8c", marginTop: 6 }}>
                          [{c.source} / {c.ref_id ?? "—"}] {c.excerpt}
                        </div>
                      ))}
                    </Card>
                  )}
                  {analysis && (
                    <Card size="small" title={`单笔分析 · ${analysis.transaction_id.slice(0, 8)}`} className="mt-3">
                      {analysis.degraded && <Tag color="orange">未配置 API Key，仅返回本地证据</Tag>}
                      {analysis.challenges.map((c, i) => (
                        <div key={i} style={{ marginTop: 8, fontSize: 13 }}>
                          <strong>· {c.claim}</strong>
                          <div style={{ color: "#595959" }}>{c.basis}</div>
                          {c.citation && <div style={{ fontSize: 12, color: "#8c8c8c" }}>出处：{c.citation.source} / {c.citation.ref_id ?? "—"}</div>}
                        </div>
                      ))}
                      {analysis.risk_notes.map((n, i) => <div key={i} style={{ marginTop: 6, fontSize: 13, color: RED }}>· {n}</div>)}
                      {analysis.raw && <div style={{ whiteSpace: "pre-wrap", marginTop: 8, fontSize: 13 }}>{analysis.raw}</div>}
                      {!analysis.challenges.length && !analysis.raw && <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无可质疑点" />}
                    </Card>
                  )}
                </div>
              ),
            },
            {
              key: "stress",
              label: "压力测试",
              children: (
                <div>
                  <div className="flex flex-wrap items-center gap-2 mb-3">
                    <Text type="secondary" style={{ fontSize: 12 }}>全部持仓</Text>
                    <InputNumber size="small" style={{ width: 100 }} value={stressGlobal} min={-90} max={200} addonAfter="%" onChange={(v) => setStressGlobal(v ?? 0)} />
                    <Text type="secondary" style={{ fontSize: 12 }}>单标的</Text>
                    <Input size="small" style={{ width: 100 }} placeholder="代码" value={stressSymbol} onChange={(e) => setStressSymbol(e.target.value)} />
                    <InputNumber size="small" style={{ width: 100 }} value={stressSymbolPct} min={-90} max={200} addonAfter="%" onChange={(v) => setStressSymbolPct(v ?? 0)} />
                    <Button size="small" type="primary" onClick={runStress}>试算</Button>
                    <Button size="small" onClick={() => fetchContractRisk([])}>合约强平距离</Button>
                    <Button size="small" onClick={fetchQuality}>数据质量体检</Button>
                  </div>
                  {stressResult && (
                    <Descriptions size="small" column={4} className="mb-3" items={[
                      { key: "a", label: "当前回撤", children: `${stressResult.baseline_drawdown.toFixed(2)}%` },
                      { key: "b", label: "假设回撤", children: `${stressResult.stressed_drawdown.toFixed(2)}%` },
                      { key: "c", label: "总资产变化", children: `${stressResult.baseline_total_assets.toFixed(2)} → ${stressResult.stressed_total_assets.toFixed(2)}` },
                      { key: "d", label: "状态", children: <Tag color={STATUS[stressResult.status]?.color}>{STATUS[stressResult.status]?.label ?? stressResult.status}</Tag> },
                    ]} />
                  )}
                  {stressResult && stressResult.findings.length > 0 && (
                    <div className="mb-3">
                      {stressResult.findings.map((f, i) => (
                        <Tag key={i} color={SEVERITY[f.severity]?.color} className="mb-1">{f.message}</Tag>
                      ))}
                    </div>
                  )}
                  {stressResult && (
                    <Table dataSource={stressResult.deltas} columns={stressColumns} rowKey="symbol" size="small" pagination={false} className="mb-4" />
                  )}
                  {contractRisks.length > 0 && (
                    <>
                      <Text strong style={{ fontSize: 13 }}>合约强平距离</Text>
                      <Table dataSource={contractRisks} columns={contractColumns} rowKey="id" size="small" pagination={false} className="mt-2 mb-4" />
                    </>
                  )}
                  {quality && (
                    <Card size="small" title={`数据质量：检查 ${quality.checked} 项，发现 ${quality.rejected} 项异常`}>
                      {quality.issues.length === 0
                        ? <Text type="secondary" style={{ fontSize: 12 }}>未发现异常价格或汇率。</Text>
                        : quality.issues.map((it, i) => <div key={i} style={{ fontSize: 12 }}><Tag color="orange">{it.kind}</Tag>{it.symbol} · {it.message}</div>)}
                    </Card>
                  )}
                </div>
              ),
            },
          ]}
        />
      </Card>

      <Modal
        title="校准净值基准"
        open={calOpen}
        okText="确认校准"
        cancelText="取消"
        onCancel={() => setCalOpen(false)}
        onOk={async () => {
          if (!calReason.trim()) {
            message.warning("请填写校准原因");
            return;
          }
          const ok = await recalibrate(calReason);
          if (ok) {
            message.success("净值基准已重置，操作已留痕");
            setCalOpen(false);
            setCalReason("");
          } else {
            message.error(useRiskStore.getState().error ?? "校准失败");
          }
        }}
      >
        <Text type="secondary" style={{ fontSize: 12 }}>
          校准会以当前总资产为新基准，单位净值与 HWM 重置为 1.0，并写入一条审计记录。仅在资产口径变更或数据修复时使用。
        </Text>
        <Input.TextArea
          className="mt-3"
          rows={3}
          placeholder="校准原因（必填）"
          value={calReason}
          onChange={(e) => setCalReason(e.target.value)}
        />
      </Modal>

      <Modal
        title={`T+30 复盘 · ${reviewTarget?.symbol ?? ""}`}
        open={!!reviewTarget}
        okText="保存复盘"
        cancelText="取消"
        onCancel={() => setReviewTarget(null)}
        onOk={async () => {
          if (!reviewTarget) return;
          const ok = await completeReview(reviewTarget.id, reviewQuality, reviewContent);
          if (ok) {
            message.success("复盘已记录");
            setReviewTarget(null);
          } else {
            message.error(useRiskStore.getState().error ?? "保存失败");
          }
        }}
      >
        {reviewTarget && (
          <div className="mb-3" style={{ fontSize: 12, color: "#595959" }}>
            <div>成交日 {reviewTarget.traded_at?.slice(0, 10) ?? "—"} · 到期 {reviewTarget.due_at?.slice(0, 10) ?? "—"}</div>
            <div>当初理由：{reviewTarget.open_note ?? "未记录"}</div>
            {reviewTarget.override_reason && <div style={{ color: RED }}>破例理由：{reviewTarget.override_reason}</div>}
          </div>
        )}
        <Select
          style={{ width: "100%", marginBottom: 12 }}
          value={reviewQuality}
          options={[
            { value: "correct", label: "决策正确：逻辑兑现" },
            { value: "wrong", label: "决策错误：逻辑未兑现或属冲动交易" },
            { value: "pending", label: "暂不下结论" },
          ]}
          onChange={setReviewQuality}
        />
        <Input.TextArea
          rows={4}
          placeholder="当初的逻辑兑现了吗？这次破例值不值？"
          value={reviewContent}
          onChange={(e) => setReviewContent(e.target.value)}
        />
      </Modal>

      <Modal
        title="写投资笔记"
        open={memoOpen}
        okText="保存并冻结快照"
        cancelText="取消"
        onCancel={() => setMemoOpen(false)}
        onOk={async () => {
          if (!memoForm.title.trim()) {
            message.warning("请填写标题");
            return;
          }
          const ok = await createMemo({
            title: memoForm.title.trim(),
            content: memoForm.content,
            tags: memoForm.tags.trim() || null,
          });
          if (ok) {
            message.success("笔记已保存，当时持仓快照已冻结");
            setMemoOpen(false);
            setMemoForm({ title: "", content: "", tags: "" });
          } else {
            message.error(useRiskStore.getState().error ?? "保存失败");
          }
        }}
      >
        <Input
          placeholder="标题"
          value={memoForm.title}
          onChange={(e) => setMemoForm((f) => ({ ...f, title: e.target.value }))}
        />
        <Input
          className="mt-2"
          placeholder="标签（逗号分隔，选填）"
          value={memoForm.tags}
          onChange={(e) => setMemoForm((f) => ({ ...f, tags: e.target.value }))}
        />
        <Input.TextArea
          className="mt-2"
          rows={6}
          placeholder="记录当时的判断、依据与不确定性…"
          value={memoForm.content}
          onChange={(e) => setMemoForm((f) => ({ ...f, content: e.target.value }))}
        />
      </Modal>
    </div>
  );
}
