import React from "react";
import { Card, Statistic, Row, Col, Tooltip, Table, Tag } from "antd";
import { InfoCircleOutlined } from "@ant-design/icons";
import type { PortfolioValuation } from "../../types";

interface Props {
  valuation: PortfolioValuation | null;
  loading?: boolean;
}

const ValuationCard: React.FC<Props> = ({ valuation, loading }) => {
  if (!valuation) return null;

  const peDisplay = valuation.weighted_pe_ttm?.toFixed(1) ?? "--";
  const peCoverage = (valuation.pe_coverage * 100).toFixed(0);

  return (
    <Card
      size="small"
      title={
        <span>
          估值概览{" "}
          <Tooltip title="加权PE仅纳入PE>0的标的；股息率为TTM股息/当前市值。数据来源：雪球">
            <InfoCircleOutlined style={{ color: "#8c8c8c", fontSize: 12 }} />
          </Tooltip>
        </span>
      }
      loading={loading}
    >
      <Row gutter={16}>
        <Col span={8}>
          <Statistic
            title={
              <span>
                加权PE(TTM){" "}
                <Tag color="green" style={{ fontSize: 10, lineHeight: "16px" }}>
                  覆盖{peCoverage}%
                </Tag>
              </span>
            }
            value={peDisplay}
            suffix={valuation.weighted_pe_ttm ? "" : ""}
            valueStyle={{ fontSize: 22, fontWeight: 700 }}
          />
        </Col>
        <Col span={8}>
          <Statistic
            title="加权股息率"
            value={valuation.weighted_dividend_yield.toFixed(1) + "%"}
            valueStyle={{ fontSize: 22, fontWeight: 700 }}
          />
        </Col>
        <Col span={8}>
          <Statistic
            title="PE覆盖标的"
            value={`${valuation.pe_count} / ${valuation.holdings.length}`}
            valueStyle={{ fontSize: 22, fontWeight: 700 }}
          />
        </Col>
      </Row>
    </Card>
  );
};

export const ValuationTable: React.FC<Props> = ({ valuation, loading }) => {
  if (!valuation) return null;

  const dataSource = valuation.holdings.map((h, i) => ({
    ...h,
    key: h.symbol,
  }));

  const columns = [
    {
      title: "代码",
      dataIndex: "symbol",
      key: "symbol",
      width: 90,
      render: (v: string) => <strong>{v}</strong>,
    },
    {
      title: "名称",
      dataIndex: "name",
      key: "name",
      ellipsis: true,
    },
    {
      title: "仓位%",
      dataIndex: "weight",
      key: "weight",
      width: 80,
      render: (v: number) => v.toFixed(1) + "%",
      sorter: (a: any, b: any) => a.weight - b.weight,
    },
    {
      title: "PE(TTM)",
      dataIndex: "pe_ttm",
      key: "pe_ttm",
      width: 90,
      render: (v: number | null) => {
        if (v == null) return <span style={{ color: "#ccc" }}>--</span>;
        const color = v < 12 ? "#16a34a" : v < 25 ? "#f59e0b" : "#e53e3e";
        return <span style={{ color, fontWeight: 600 }}>{v.toFixed(1)}</span>;
      },
      sorter: (a: any, b: any) => (a.pe_ttm ?? -999) - (b.pe_ttm ?? -999),
    },
    {
      title: "股息率",
      dataIndex: "dividend_yield",
      key: "dividend_yield",
      width: 90,
      render: (v: number | null) => {
        if (v == null) return <span style={{ color: "#ccc" }}>--</span>;
        return <span style={{ color: "#16a34a", fontWeight: 600 }}>{v.toFixed(1)}%</span>;
      },
      sorter: (a: any, b: any) => (a.dividend_yield ?? -1) - (b.dividend_yield ?? -1),
    },
    {
      title: "市值",
      dataIndex: "market_value",
      key: "market_value",
      width: 100,
      align: "right" as const,
      render: (v: number) => "$" + v.toFixed(0),
      sorter: (a: any, b: any) => a.market_value - b.market_value,
    },
  ];

  return (
    <Table
      dataSource={dataSource}
      columns={columns}
      pagination={false}
      size="small"
      loading={loading}
      scroll={{ x: 550 }}
      summary={() => (
        <Table.Summary.Row style={{ fontWeight: 700, background: "#fafafa" }}>
          <Table.Summary.Cell index={0}>合计</Table.Summary.Cell>
          <Table.Summary.Cell index={1}>{valuation.holdings.length} 个持仓</Table.Summary.Cell>
          <Table.Summary.Cell index={2}>100%</Table.Summary.Cell>
          <Table.Summary.Cell index={3}>{valuation.weighted_pe_ttm?.toFixed(1) ?? "--"}</Table.Summary.Cell>
          <Table.Summary.Cell index={4}>{valuation.weighted_dividend_yield.toFixed(1)}%</Table.Summary.Cell>
          <Table.Summary.Cell index={5}>${valuation.total_market_value.toFixed(0)}</Table.Summary.Cell>
        </Table.Summary.Row>
      )}
    />
  );
};

export default ValuationCard;
