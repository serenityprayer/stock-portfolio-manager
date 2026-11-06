import { useEffect, useState } from "react";
import {
  Alert,
  Button,
  Card,
  DatePicker,
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
  Typography,
  message,
} from "antd";
import { ReloadOutlined } from "@ant-design/icons";
import dayjs from "dayjs";
import { invoke } from "@tauri-apps/api/core";
import { useRiskStore } from "../../stores/riskStore";
import type {
  ExternalCashFlow,
  RiskEvent,
  RiskRuleConfig,
  RiskSeverity,
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

  useEffect(() => {
    fetchDashboard();
    fetchFlows();
    fetchHwm();
    fetchWatchlist();
    fetchStats();
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
    </div>
  );
}
