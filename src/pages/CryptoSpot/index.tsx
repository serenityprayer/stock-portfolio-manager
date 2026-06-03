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
  Popconfirm,
  message,
} from "antd";
import { PlusOutlined, DeleteOutlined, EditOutlined } from "@ant-design/icons";
import { useCryptoSpotStore } from "../../stores/cryptoSpotStore";
import type { CryptoSpot } from "../../types";
import { invoke } from "@tauri-apps/api/core";

const { Title, Text } = Typography;

export default function CryptoSpotPage() {
  const {
    cryptoSpots,
    loading,
    fetchCryptoSpots,
    createCryptoSpot,
    updateCryptoSpot,
    deleteCryptoSpot,
  } = useCryptoSpotStore();

  const [modalOpen, setModalOpen] = useState(false);
  const [editingSpot, setEditingSpot] = useState<CryptoSpot | null>(null);
  const [form] = Form.useForm();
  const [quotes, setQuotes] = useState<Record<string, { price: number }>>({});

  useEffect(() => {
    fetchCryptoSpots();
  }, [fetchCryptoSpots]);

  // 拉取行情
  useEffect(() => {
    if (cryptoSpots.length === 0) return;
    const symbols = cryptoSpots.map((s) => s.symbol).join(",");
    invoke<Record<string, { price: number; change: number; changePercent: number; high: number; low: number; volume: number }>>("fetch_crypto_quotes", {
      symbols,
    })
      .then((data) => {
        console.log("[CryptoSpot] quotes response:", data);
        setQuotes(data);
      })
      .catch((err) => {
        console.error("[CryptoSpot] 获取行情失败:", err);
        message.error(`获取行情失败: ${err}`);
      });
  }, [cryptoSpots]);

  const handleSubmit = async (values: {
    symbol: string;
    name?: string;
    buy_price: number;
    shares: number;
    fee?: number;
    exchange?: string;
    notes?: string;
  }) => {
    try {
      if (editingSpot) {
        await updateCryptoSpot({ id: editingSpot.id, ...values });
        message.success("修改成功");
      } else {
        await createCryptoSpot(values);
        message.success("添加成功");
      }
      setModalOpen(false);
      form.resetFields();
      setEditingSpot(null);
    } catch (err) {
      message.error(`操作失败: ${err}`);
    }
  };

  const handleEdit = (spot: CryptoSpot) => {
    setEditingSpot(spot);
    form.setFieldsValue({
      symbol: spot.symbol,
      name: spot.name,
      buy_price: spot.buy_price,
      shares: spot.shares,
      fee: spot.fee,
      exchange: spot.exchange,
      notes: spot.notes,
    });
    setModalOpen(true);
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteCryptoSpot(id);
      message.success("删除成功");
    } catch (err) {
      message.error(`删除失败: ${err}`);
    }
  };

  const columns = [
    {
      title: "币种",
      dataIndex: "symbol",
      key: "symbol",
      render: (symbol: string, record: CryptoSpot) => (
        <Space>
          <Text strong>{symbol}</Text>
          {record.name && <Text type="secondary">{record.name}</Text>}
        </Space>
      ),
    },
    {
      title: "持仓量",
      dataIndex: "shares",
      key: "shares",
      render: (v: number) => v.toFixed(4),
    },
    {
      title: "成本价",
      dataIndex: "buy_price",
      key: "buy_price",
      render: (v: number) => `$${v.toFixed(2)}`,
    },
    {
      title: "现价",
      key: "current_price",
      render: (_: unknown, record: CryptoSpot) =>
        quotes[record.symbol]
          ? `$${quotes[record.symbol].price.toFixed(2)}`
          : "-",
    },
    {
      title: "市值",
      key: "market_value",
      render: (_: unknown, record: CryptoSpot) =>
        quotes[record.symbol]
          ? `$${(quotes[record.symbol].price * record.shares).toFixed(2)}`
          : "-",
    },
    {
      title: "盈亏",
      key: "pnl",
      render: (_: unknown, record: CryptoSpot) => {
        const q = quotes[record.symbol];
        if (!q) return "-";
        const pnl = (q.price - record.buy_price) * record.shares - record.fee;
        const color = pnl >= 0 ? "red" : "green";
        return <Text style={{ color }}>{pnl >= 0 ? "+" : ""}{pnl.toFixed(2)}</Text>;
      },
    },
    {
      title: "盈亏%",
      key: "pnl_pct",
      render: (_: unknown, record: CryptoSpot) => {
        const q = quotes[record.symbol];
        if (!q) return "-";
        const pnlPct = ((q.price - record.buy_price) / record.buy_price) * 100;
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
      render: (_: unknown, record: CryptoSpot) => (
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

  // 统计
  const totalMarketValue = cryptoSpots.reduce((sum, s) => {
    const q = quotes[s.symbol];
    return sum + (q ? q.price * s.shares : 0);
  }, 0);
  const totalCost = cryptoSpots.reduce((sum, s) => sum + s.buy_price * s.shares + s.fee, 0);
  const totalPnl = totalMarketValue - totalCost;

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <Title level={2} className="!mb-0">💰 数字货币现货</Title>
        <Button
          type="primary"
          icon={<PlusOutlined />}
          onClick={() => {
            setEditingSpot(null);
            form.resetFields();
            setModalOpen(true);
          }}
        >
          添加现货
        </Button>
      </div>

      {/* 统计卡片 */}
      <div className="grid grid-cols-3 gap-4 mb-4">
        <div className="bg-white p-4 rounded shadow">
          <Text type="secondary">持仓市值</Text>
          <div className="text-2xl font-bold">${totalMarketValue.toFixed(2)}</div>
        </div>
        <div className="bg-white p-4 rounded shadow">
          <Text type="secondary">总成本</Text>
          <div className="text-2xl font-bold">${totalCost.toFixed(2)}</div>
        </div>
        <div className="bg-white p-4 rounded shadow">
          <Text type="secondary">未实现盈亏</Text>
          <div className={`text-2xl font-bold ${totalPnl >= 0 ? "text-red-500" : "text-green-500"}`}>
            {totalPnl >= 0 ? "+" : ""}${totalPnl.toFixed(2)}
          </div>
        </div>
      </div>

      <Table
        dataSource={cryptoSpots}
        columns={columns}
        rowKey="id"
        loading={loading}
        pagination={false}
      />

      <Modal
        title={editingSpot ? "编辑现货" : "添加现货"}
        open={modalOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setModalOpen(false);
          setEditingSpot(null);
          form.resetFields();
        }}
        okText="确认"
        cancelText="取消"
      >
        <Form form={form} layout="vertical" onFinish={handleSubmit}>
          <Form.Item name="symbol" label="币种代码" rules={[{ required: true, message: "请输入币种代码" }]}>
            <Input placeholder="如：BTC, ETH, SOL" />
          </Form.Item>
          <Form.Item name="name" label="币种名称">
            <Input placeholder="如：Bitcoin, Ethereum" />
          </Form.Item>
          <Form.Item name="buy_price" label="买入均价（USD）" rules={[{ required: true, message: "请输入买入均价" }]}>
            <InputNumber min={0} step={0.01} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="shares" label="持仓数量" rules={[{ required: true, message: "请输入持仓数量" }]}>
            <InputNumber min={0} step={0.0001} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="fee" label="手续费（USD）" initialValue={0}>
            <InputNumber min={0} step={0.01} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="exchange" label="交易所" initialValue="Binance">
            <Input placeholder="Binance, OKX, Bybit" />
          </Form.Item>
          <Form.Item name="notes" label="备注">
            <Input.TextArea rows={2} />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
