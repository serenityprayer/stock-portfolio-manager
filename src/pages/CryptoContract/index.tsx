import { useEffect, useState } from "react";
import {
  Typography,
  Button,
  Table,
  Space,
  Modal,
  Form,
  InputNumber,
  Input,
  Select,
  Popconfirm,
  message,
  Tag,
  Radio,
  Tabs,
  Descriptions,
  Statistic,
} from "antd";
import { PlusOutlined, DeleteOutlined, EditOutlined, CloseOutlined } from "@ant-design/icons";
import { useCryptoContractStore } from "../../stores/cryptoContractStore";
import type { CryptoContract, ContractHistory } from "../../types";

const { Title, Text } = Typography;

export default function CryptoContractPage() {
  const {
    contracts,
    contractHistory,
    loading,
    fetchContracts,
    fetchContractHistory,
    createContract,
    updateContract,
    addPosition,
    closeContract,
    deleteContract,
    fetchQuotes,
  } = useCryptoContractStore();
  const [quotes, setQuotes] = useState<Record<string, { price: number; change: number; changePercent: number }>>({});
  const [modalOpen, setModalOpen] = useState(false);
  const [editingContract, setEditingContract] = useState<CryptoContract | null>(null);
  const [form] = Form.useForm();

  // 平仓相关 state
  const [closeModalOpen, setCloseModalOpen] = useState(false);
  const [closingContract, setClosingContract] = useState<CryptoContract | null>(null);
  const [closeShares, setCloseShares] = useState<number>(0);
  const [closePrice, setClosePrice] = useState<number>(0);

  // 加仓相关 state
  const [addModalOpen, setAddModalOpen] = useState(false);
  const [addingContract, setAddingContract] = useState<CryptoContract | null>(null);
  const [addShares, setAddShares] = useState<number>(0);
  const [addPrice, setAddPrice] = useState<number>(0);
  const [addFee, setAddFee] = useState<number>(0);
  const [activeTab, setActiveTab] = useState<string>("active");

  useEffect(() => {
    fetchContracts();
    fetchContractHistory();
  }, [fetchContracts, fetchContractHistory]);

  // 拉取行情（按 asset_type 分流）
  const loadQuotes = async () => {
    try {
      const data = await fetchQuotes();
      setQuotes(data);
    } catch (err) {
      console.error("[CryptoContract] 获取行情失败:", err);
    }
  };

  useEffect(() => {
    if (contracts.length === 0) return;
    loadQuotes();
    const timer = setInterval(loadQuotes, 30000);
    return () => clearInterval(timer);
  }, [contracts]);

  const handleSubmit = async (values: {
    symbol: string;
    name?: string;
    asset_type?: "crypto" | "tradfi";
    position_type: "long" | "short";
    open_price: number;
    shares: number;
    leverage: number;
    fee?: number;
    exchange?: string;
    notes?: string;
  }) => {
    const symbolUpper = values.symbol.trim().toUpperCase();
    const assetType = values.asset_type || "crypto";

    try {
      if (editingContract) {
        await updateContract({
          id: editingContract.id,
          symbol: symbolUpper,
          asset_type: assetType,
          position_type: values.position_type,
          open_price: values.open_price,
          shares: values.shares,
          leverage: values.leverage,
          fee: values.fee,
          exchange: values.exchange,
          notes: values.notes,
        });
        message.success("修改成功");
      } else {
        await createContract({
          symbol: symbolUpper,
          name: values.name,
          asset_type: assetType,
          position_type: values.position_type,
          open_price: values.open_price,
          shares: values.shares,
          leverage: values.leverage,
          fee: values.fee,
          exchange: values.exchange,
          notes: values.notes,
        });
        message.success("添加成功");
      }
      setModalOpen(false);
      form.resetFields();
      setEditingContract(null);
    } catch (err) {
      message.error(`操作失败: ${err}`);
    }
  };

  const handleEdit = (contract: CryptoContract) => {
    setEditingContract(contract);
    form.setFieldsValue({
      symbol: contract.symbol,
      name: contract.name,
      asset_type: contract.asset_type || "crypto",
      position_type: contract.position_type,
      open_price: contract.open_price,
      shares: contract.shares,
      leverage: contract.leverage,
      fee: contract.fee,
      exchange: contract.exchange,
      notes: contract.notes,
    });
    setModalOpen(true);
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteContract(id);
      message.success("删除成功");
    } catch (err) {
      message.error(`删除失败: ${err}`);
    }
  };

  // 打平仓
  const handleOpenCloseModal = (contract: CryptoContract) => {
    setClosingContract(contract);
    setCloseShares(contract.shares);
    const q = quotes[contract.symbol];
    setClosePrice(q ? q.price : contract.open_price);
    setCloseModalOpen(true);
  };

  const handleCloseContract = async () => {
    if (!closingContract) return;
    try {
      await closeContract({
        id: closingContract.id,
        closePrice: closePrice,
        closeShares: closeShares,
        closeFee: 0,
      });
      message.success(
        closeShares >= closingContract.shares - 1e-8
          ? "已全部平仓"
          : `已平仓 ${closeShares}，剩余 ${(closingContract.shares - closeShares).toFixed(4)}`
      );
      setCloseModalOpen(false);
      setClosingContract(null);
    } catch (err) {
      message.error(`平仓失败: ${err}`);
    }
  };

  // 加仓
  const handleOpenAddModal = (contract: CryptoContract) => {
    setAddingContract(contract);
    setAddShares(0);
    const q = quotes[contract.symbol];
    setAddPrice(q ? q.price : contract.open_price);
    setAddFee(0);
    setAddModalOpen(true);
  };

  const handleAddPosition = async () => {
    if (!addingContract) return;
    try {
      await addPosition({
        id: addingContract.id,
        addShares,
        addPrice: addPrice,
        addFee: addFee,
      });
      message.success("加仓成功，已重新计算平均开仓价");
      setAddModalOpen(false);
      setAddingContract(null);
    } catch (err) {
      message.error(`加仓失败: ${err}`);
    }
  };

  // 计算加仓后预览
  const previewAvgPrice = addingContract
    ? (
        (addingContract.open_price * addingContract.shares + addPrice * addShares) /
        (addingContract.shares + addShares)
      ).toFixed(2)
    : "-";
  const previewPnl = closingContract
    ? (() => {
        const direction = closingContract.position_type === "long" ? 1 : -1;
        return (closePrice - closingContract.open_price) * closeShares * direction;
      })()
    : 0;

  const columns = [
    {
      title: "标的",
      dataIndex: "symbol",
      key: "symbol",
      render: (symbol: string, record: CryptoContract) => (
        <Space>
          <Tag color={record.position_type === "long" ? "red" : "green"}>
            {record.position_type === "long" ? "多" : "空"}
          </Tag>
          <Tag color={record.asset_type === "crypto" ? "blue" : "purple"}>
            {record.asset_type === "crypto" ? "加密" : "TradFi"}
          </Tag>
          <Text strong>{symbol}</Text>
          {record.name && <Text type="secondary">{record.name}</Text>}
        </Space>
      ),
    },
    {
      title: "方向",
      dataIndex: "position_type",
      key: "position_type",
      render: (v: string) => (
        <Tag color={v === "long" ? "red" : "green"}>
          {v === "long" ? "做多" : "做空"}
        </Tag>
      ),
    },
    {
      title: "开仓价",
      dataIndex: "open_price",
      key: "open_price",
      render: (v: number) => `$${v.toFixed(2)}`,
    },
    {
      title: "现价",
      key: "current_price",
      render: (_: unknown, record: CryptoContract) =>
        quotes[record.symbol]
          ? `$${quotes[record.symbol].price.toFixed(2)}`
          : "-",
    },
    {
      title: "数量",
      dataIndex: "shares",
      key: "shares",
      render: (v: number) => v.toFixed(4),
    },
    {
      title: "杠杆",
      dataIndex: "leverage",
      key: "leverage",
      render: (v: number) => `${v}x`,
    },
    {
      title: "保证金",
      dataIndex: "margin",
      key: "margin",
      render: (v: number) => `$${v.toFixed(2)}`,
    },
    {
      title: "强平价格",
      dataIndex: "liquidation_price",
      key: "liquidation_price",
      render: (v: number | null) =>
        v ? `$${v.toFixed(2)}` : "-",
    },
    {
      title: "未实现盈亏",
      key: "pnl",
      render: (_: unknown, record: CryptoContract) => {
        const q = quotes[record.symbol];
        if (!q) return "-";
        const pnl = record.position_type === "long"
          ? (q.price - record.open_price) * record.shares
          : (record.open_price - q.price) * record.shares;
        const color = pnl >= 0 ? "red" : "green";
        return <Text style={{ color }}>{pnl >= 0 ? "+" : ""}{pnl.toFixed(2)}</Text>;
      },
    },
    {
      title: "操作",
      key: "action",
      render: (_: unknown, record: CryptoContract) => (
        <Space>
          <Button type="link" size="small" onClick={() => handleOpenAddModal(record)}>
            加仓
          </Button>
          <Button type="link" size="small" onClick={() => handleOpenCloseModal(record)}>
            平仓
          </Button>
          <Button type="link" size="small" onClick={() => handleEdit(record)}>
            编辑
          </Button>
          <Popconfirm
            title="确认删除？"
            onConfirm={() => handleDelete(record.id)}
            okText="确认"
            cancelText="取消"
          >
            <Button type="link" size="small" danger>
              删除
            </Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  // 成交历史列
  const historyColumns = [
    {
      title: "类型",
      dataIndex: "action_type",
      key: "action_type",
      width: 80,
      render: (v: string) => (
        <Tag color={v === "close" ? "orange" : "blue"}>
          {v === "close" ? "平仓" : "加仓"}
        </Tag>
      ),
    },
    {
      title: "标的",
      dataIndex: "symbol",
      key: "symbol",
      render: (symbol: string, record: Record<string, unknown>) => (
        <Space>
          <Tag color={record.asset_type === "crypto" ? "blue" : "purple"}>
            {record.asset_type === "crypto" ? "加密" : "TradFi"}
          </Tag>
          <Text strong>{symbol}</Text>
          {record.name != null && <Text type="secondary">{(record.name as string)}</Text>}
        </Space>
      ),
    },
    {
      title: "方向",
      dataIndex: "position_type",
      key: "position_type",
      render: (v: string) => (
        <Tag color={v === "long" ? "red" : "green"}>
          {v === "long" ? "做多" : "做空"}
        </Tag>
      ),
    },
    {
      title: "平仓价",
      key: "close_price",
      render: (_: unknown, record: Record<string, unknown>) =>
        record.action_type === "close" && record.close_price != null
          ? `$${(record.close_price as number).toFixed(2)}`
          : "-",
    },
    {
      title: "平仓数量",
      key: "close_shares",
      render: (_: unknown, record: Record<string, unknown>) =>
        record.action_type === "close" && record.close_shares != null
          ? (record.close_shares as number).toFixed(4)
          : "-",
    },
    {
      title: "加仓价",
      key: "add_price",
      render: (_: unknown, record: Record<string, unknown>) =>
        record.action_type === "add" && record.add_price != null
          ? `$${(record.add_price as number).toFixed(2)}`
          : "-",
    },
    {
      title: "加仓数量",
      key: "add_shares",
      render: (_: unknown, record: Record<string, unknown>) =>
        record.action_type === "add" && record.add_shares != null
          ? `+${(record.add_shares as number).toFixed(4)}`
          : "-",
    },
    {
      title: "新均价",
      key: "new_avg_price",
      render: (_: unknown, record: Record<string, unknown>) =>
        record.new_avg_price != null
          ? `$${(record.new_avg_price as number).toFixed(2)}`
          : "-",
    },
    {
      title: "新总仓位",
      key: "new_total_shares",
      render: (_: unknown, record: Record<string, unknown>) =>
        record.new_total_shares != null
          ? (record.new_total_shares as number).toFixed(4)
          : "-",
    },
    {
      title: "已实现盈亏",
      key: "realized_pnl",
      render: (_: unknown, record: Record<string, unknown>) => {
        if (record.action_type !== "close" || record.realized_pnl == null) return "-";
        const v = record.realized_pnl as number;
        const color = v >= 0 ? "red" : "green";
        return <Text style={{ color }}>{v >= 0 ? "+" : ""}{v.toFixed(2)}</Text>;
      },
    },
    {
      title: "回报率",
      key: "return_rate",
      render: (_: unknown, record: Record<string, unknown>) => {
        if (record.action_type !== "close" || record.return_rate == null) return "-";
        const v = record.return_rate as number;
        const color = v >= 0 ? "red" : "green";
        return <Text style={{ color }}>{v >= 0 ? "+" : ""}{(v * 100).toFixed(2)}%</Text>;
      },
    },
    {
      title: "杠杆",
      dataIndex: "leverage",
      key: "leverage",
      render: (v: number) => `${v}x`,
    },
    {
      title: "时间",
      dataIndex: "closed_at",
      key: "closed_at",
      render: (v: string) => v?.slice(0, 19)?.replace("T", " "),
    },
  ];

  const totalMargin = contracts.reduce((sum, c) => sum + c.margin, 0);
  const totalPnl = contracts.reduce((sum, c) => {
    const q = quotes[c.symbol];
    if (!q) return sum;
    const pnl = c.position_type === "long"
      ? (q.price - c.open_price) * c.shares
      : (c.open_price - q.price) * c.shares;
    return sum + pnl;
  }, 0);

  return (
    <div>
      <Tabs
        activeKey={activeTab}
        onChange={setActiveTab}
        items={[
          {
            key: "active",
            label: `持仓中 (${contracts.length})`,
            children: (
              <div>
                <div className="flex justify-between items-center mb-4">
                  <Title level={2} className="!mb-0">⚡ 合约持仓</Title>
                  <Button
                    type="primary"
                    icon={<PlusOutlined />}
                    onClick={() => {
                      setEditingContract(null);
                      form.resetFields();
                      form.setFieldsValue({ asset_type: "crypto", position_type: "long", leverage: 1 });
                      setModalOpen(true);
                    }}
                  >
                    添加合约
                  </Button>
                </div>

                <div className="grid grid-cols-3 gap-4 mb-4">
                  <div className="bg-white p-4 rounded shadow">
                    <Text type="secondary">总保证金</Text>
                    <div className="text-2xl font-bold">${totalMargin.toFixed(2)}</div>
                  </div>
                  <div className="bg-white p-4 rounded shadow">
                    <Text type="secondary">未实现盈亏</Text>
                    <div className={`text-2xl font-bold ${totalPnl >= 0 ? "text-red-500" : "text-green-500"}`}>
                      {totalPnl >= 0 ? "+" : ""}${totalPnl.toFixed(2)}
                    </div>
                  </div>
                  <div className="bg-white p-4 rounded shadow">
                    <Text type="secondary">持仓数量</Text>
                    <div className="text-2xl font-bold">{contracts.length}</div>
                  </div>
                </div>

                <Table
                  dataSource={contracts}
                  columns={columns}
                  rowKey="id"
                  loading={loading}
                  pagination={false}
                />
              </div>
            ),
          },
          {
            key: "history",
            label: `成交历史 (${contractHistory.length})`,
            children: (
              <Table
                dataSource={contractHistory as any[]}
                columns={historyColumns}
                rowKey="id"
                pagination={{ pageSize: 20 }}
              />
            ),
          },
        ]}
      />

      {/* 添加/编辑弹窗 */}
      <Modal
        title={editingContract ? "编辑合约" : "添加合约"}
        open={modalOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setModalOpen(false);
          setEditingContract(null);
          form.resetFields();
        }}
        okText="确认"
        cancelText="取消"
        width={600}
      >
        <Form form={form} layout="vertical" onFinish={handleSubmit}>
          <Form.Item name="asset_type" label="资产类型" rules={[{ required: true }]}>
            <Radio.Group>
              <Radio value="crypto">加密货币（BTC/ETH）</Radio>
              <Radio value="tradfi">TradFi（HOOD/AAPL）</Radio>
            </Radio.Group>
          </Form.Item>
          <Form.Item name="symbol" label="标的代码" rules={[{ required: true, message: "请输入代码" }]}>
            <Input placeholder="BTC（加密）或 HOOD（TradFi）" />
          </Form.Item>
          <Form.Item name="name" label="名称">
            <Input placeholder="Bitcoin 或 Robinhood" />
          </Form.Item>
          <Form.Item name="position_type" label="持仓方向" rules={[{ required: true }]}>
            <Select>
              <Select.Option value="long">做多（Long）</Select.Option>
              <Select.Option value="short">做空（Short）</Select.Option>
            </Select>
          </Form.Item>
          <Form.Item name="open_price" label="开仓价（USD）" rules={[{ required: true, message: "请输入开仓价" }]}>
            <InputNumber min={0} step={0.01} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="shares" label="持仓数量" rules={[{ required: true, message: "请输入数量" }]}>
            <InputNumber min={0} step={0.0001} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="leverage" label="杠杆倍数" rules={[{ required: true }]} tooltip="TradFi 杠杆为1（无杠杆）">
            <InputNumber min={1} max={125} step={1} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="fee" label="手续费（USD）" initialValue={0}>
            <InputNumber min={0} step={0.01} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="exchange" label="交易所/券商" tooltip="Binance/Coinbase（加密）或 IBKR/TDA（TradFi）">
            <Input placeholder="Binance / IBKR" />
          </Form.Item>
          <Form.Item name="notes" label="备注">
            <Input.TextArea rows={2} />
          </Form.Item>
        </Form>
      </Modal>

      {/* 平仓弹窗 */}
      <Modal
        title="合约平仓"
        open={closeModalOpen}
        onOk={handleCloseContract}
        onCancel={() => {
          setCloseModalOpen(false);
          setClosingContract(null);
        }}
        okText="确认平仓"
        cancelText="取消"
        okButtonProps={{ danger: true }}
        width={500}
      >
        {closingContract && (
          <div>
            <Descriptions column={2} size="small" bordered className="mb-4">
              <Descriptions.Item label="标的">{closingContract.symbol}</Descriptions.Item>
              <Descriptions.Item label="方向">
                <Tag color={closingContract.position_type === "long" ? "red" : "green"}>
                  {closingContract.position_type === "long" ? "做多" : "做空"}
                </Tag>
              </Descriptions.Item>
              <Descriptions.Item label="开仓价">${closingContract.open_price.toFixed(2)}</Descriptions.Item>
              <Descriptions.Item label="持仓数量">{closingContract.shares.toFixed(4)}</Descriptions.Item>
              <Descriptions.Item label="杠杆">{closingContract.leverage}x</Descriptions.Item>
              <Descriptions.Item label="现价">
                ${quotes[closingContract.symbol]?.price.toFixed(2) ?? "-"}
              </Descriptions.Item>
            </Descriptions>

            <div className="space-y-3 mt-4">
              <div>
                <Text>平仓数量：</Text>
                <InputNumber
                  min={0.0001}
                  max={closingContract.shares}
                  step={closingContract.shares < 1 ? 0.0001 : 1}
                  value={closeShares}
                  onChange={(v) => setCloseShares(v ?? 0)}
                  style={{ width: "100%" }}
                />
              </div>
              <div>
                <Text>平仓价格：</Text>
                <InputNumber
                  min={0}
                  step={0.01}
                  value={closePrice}
                  onChange={(v) => setClosePrice(v ?? 0)}
                  style={{ width: "100%" }}
                  addonAfter="$"
                />
              </div>
              <div className="p-3 bg-gray-50 rounded">
                <Statistic
                  title="预估盈亏"
                  value={previewPnl}
                  precision={2}
                  prefix={previewPnl >= 0 ? "+$" : "-$"}
                  valueStyle={{ color: previewPnl >= 0 ? "#f5222d" : "#52c41a" }}
                />
              </div>
            </div>
          </div>
        )}
      </Modal>

      {/* 加仓弹窗 */}
      <Modal
        title="加仓"
        open={addModalOpen}
        onOk={handleAddPosition}
        onCancel={() => {
          setAddModalOpen(false);
          setAddingContract(null);
        }}
        okText="确认加仓"
        cancelText="取消"
        width={500}
      >
        {addingContract && (
          <div>
            <Descriptions column={2} size="small" bordered className="mb-4">
              <Descriptions.Item label="标的">{addingContract.symbol}</Descriptions.Item>
              <Descriptions.Item label="方向">
                <Tag color={addingContract.position_type === "long" ? "red" : "green"}>
                  {addingContract.position_type === "long" ? "做多" : "做空"}
                </Tag>
              </Descriptions.Item>
              <Descriptions.Item label="当前开仓价">${addingContract.open_price.toFixed(2)}</Descriptions.Item>
              <Descriptions.Item label="当前数量">{addingContract.shares.toFixed(4)}</Descriptions.Item>
              <Descriptions.Item label="杠杆">{addingContract.leverage}x</Descriptions.Item>
            </Descriptions>

            <div className="space-y-3 mt-4">
              <div>
                <Text>加仓数量：</Text>
                <InputNumber
                  min={0.0001}
                  step={addingContract.shares < 1 ? 0.0001 : 1}
                  value={addShares}
                  onChange={(v) => setAddShares(v ?? 0)}
                  style={{ width: "100%" }}
                />
              </div>
              <div>
                <Text>加仓价格：</Text>
                <InputNumber
                  min={0}
                  step={0.01}
                  value={addPrice}
                  onChange={(v) => setAddPrice(v ?? 0)}
                  style={{ width: "100%" }}
                  addonAfter="$"
                />
              </div>
              <div>
                <Text>手续费（USD）：</Text>
                <InputNumber
                  min={0}
                  step={0.01}
                  value={addFee}
                  onChange={(v) => setAddFee(v ?? 0)}
                  style={{ width: "100%" }}
                />
              </div>
              <div className="p-3 bg-gray-50 rounded">
                <Text type="secondary">加仓后加权平均开仓价：</Text>
                <div className="text-lg font-bold">${previewAvgPrice}</div>
              </div>
            </div>
          </div>
        )}
      </Modal>
    </div>
  );
}
