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
} from "antd";
import { PlusOutlined, DeleteOutlined, EditOutlined } from "@ant-design/icons";
import { useCryptoContractStore } from "../../stores/cryptoContractStore";
import type { CryptoContract } from "../../types";
import { invoke } from "@tauri-apps/api/core";

const { Title, Text } = Typography;

export default function CryptoContractPage() {
  const {
    contracts,
    loading,
    fetchContracts,
    createContract,
    updateContract,
    deleteContract,
    fetchQuotes,
  } = useCryptoContractStore();
  const [quotes, setQuotes] = useState<Record<string, { price: number; change: number; changePercent: number }>>({});
  const [modalOpen, setModalOpen] = useState(false);
  const [editingContract, setEditingContract] = useState<CryptoContract | null>(null);
  const [form] = Form.useForm();

  useEffect(() => {
    fetchContracts();
  }, [fetchContracts]);

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

    // 校验行情接口是否可访问
    try {
      const checkCmd = assetType === "crypto" ? "fetch_crypto_quotes" : "fetch_tradfi_quotes";
      const check = await invoke<Record<string, { price: number }>>(checkCmd, {
        symbols: symbolUpper,
      });
      if (!check[symbolUpper] || check[symbolUpper].price <= 0) {
        message.error(`标的 ${symbolUpper} 无法获取行情，请检查代码`);
        return;
      }
    } catch (err) {
      message.error(`标的 ${symbolUpper} 无效: ${err}`);
      return;
    }

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
      title: "收益率%",
      key: "pnl_pct",
      render: (_: unknown, record: CryptoContract) => {
        const q = quotes[record.symbol];
        if (!q) return "-";
        const pnlPct = record.position_type === "long"
          ? ((q.price - record.open_price) / record.open_price) * 100 * record.leverage
          : ((record.open_price - q.price) / record.open_price) * 100 * record.leverage;
        const color = pnlPct >= 0 ? "red" : "green";
        return <Text style={{ color }}>{pnlPct >= 0 ? "+" : ""}{pnlPct.toFixed(2)}%</Text>;
      },
    },
    {
      title: "交易所",
      dataIndex: "exchange",
      key: "exchange",
    },
    {
      title: "操作",
      key: "action",
      render: (_: unknown, record: CryptoContract) => (
        <Space>
          <Button type="link" size="small" onClick={() => handleEdit(record)} icon={<EditOutlined />}>
            编辑
          </Button>
          <Popconfirm
            title="确认删除？"
            onConfirm={() => handleDelete(record.id)}
            okText="确认"
            cancelText="取消"
          >
            <Button type="link" size="small" danger icon={<DeleteOutlined />}>
              删除
            </Button>
          </Popconfirm>
        </Space>
      ),
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
    </div>
  );
}
