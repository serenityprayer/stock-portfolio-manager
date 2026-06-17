import { Row, Col, Card, Statistic, Spin, Empty, Collapse } from "antd";
import PieChart from "../../components/charts/PieChart";
import BarChart from "../../components/charts/BarChart";
import ValuationCard, { ValuationTable } from "../../components/ValuationCard";
import type { StatisticsOverview, PortfolioValuation } from "../../types";
import type { Currency } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

const currencySymbol: Record<string, string> = {
  USD: "$",
  CNY: "¥",
  HKD: "HK$",
};

interface Props {
  overview: StatisticsOverview | null;
  loading: boolean;
  baseCurrency: Currency;
  market?: string;
  accountId?: string;
  categoryId?: string;
}

export default function OverviewTab({ overview, loading, baseCurrency, market, accountId, categoryId }: Props) {
  const { pnlColor } = usePnlColor();
  const symbol = currencySymbol[baseCurrency] ?? "$";
  const [valuation, setValuation] = useState<PortfolioValuation | null>(null);
  const [valLoading, setValLoading] = useState(false);

  useEffect(() => {
    setValLoading(true);
    invoke<PortfolioValuation>("get_portfolio_valuation", {
      market: market ?? null,
      accountId: accountId ?? null,
      categoryId: categoryId ?? null,
    })
      .then(setValuation)
      .catch((e) => console.warn("get_portfolio_valuation error:", e))
      .finally(() => setValLoading(false));
  }, [market, accountId, categoryId]);

  if (loading && !overview) {
    return (
      <div className="flex justify-center py-16">
        <Spin size="large" />
      </div>
    );
  }
  if (!overview) {
    return <Empty description="暂无数据" />;
  }

  const totalPnlPos = overview.total_pnl >= 0;

  const gainersData = overview.top_gainers.map((g) => ({
    name: g.symbol,
    value: parseFloat(g.pnl.toFixed(2)),
  }));
  const losersData = overview.top_losers.map((g) => ({
    name: g.symbol,
    value: parseFloat(g.pnl.toFixed(2)),
  }));

  return (
    <div>
      {/* Summary stats */}
      <Row gutter={[16, 16]} className="mb-4">
        <Col xs={24} sm={6}>
          <Card>
            <Statistic
              title={`总市值 (${baseCurrency})`}
              value={overview.total_market_value.toFixed(2)}
              prefix={symbol}
            />
          </Card>
        </Col>
        <Col xs={24} sm={6}>
          <Card>
            <Statistic
              title={`总成本 (${baseCurrency})`}
              value={overview.total_cost.toFixed(2)}
              prefix={symbol}
            />
          </Card>
        </Col>
        <Col xs={24} sm={6}>
          <Card>
            <Statistic
              title={`总盈亏 (${baseCurrency})`}
              value={`${totalPnlPos ? "+" : ""}${overview.total_pnl.toFixed(2)}`}
              valueStyle={{ color: pnlColor(overview.total_pnl) }}
              prefix={symbol}
              suffix={`(${totalPnlPos ? "+" : ""}${overview.total_pnl_percent.toFixed(2)}%)`}
            />
          </Card>
        </Col>
        <Col xs={24} sm={6}>
          {valuation ? (
            <Card>
              <Statistic
                title="加权PE(TTM)"
                value={valuation.weighted_pe_ttm?.toFixed(1) ?? "--"}
                valueStyle={{ fontSize: 22, fontWeight: 700 }}
                suffix={
                  <span style={{ fontSize: 11, color: "#8c8c8c" }}>
                    覆盖{(valuation.pe_coverage * 100).toFixed(0)}%
                  </span>
                }
              />
            </Card>
          ) : (
            <Card loading={valLoading}>
              <Statistic title="加权PE(TTM)" value="--" />
            </Card>
          )}
        </Col>
      </Row>

      {/* Valuation detail table */}
      {valuation && valuation.holdings.length > 0 && (
        <div className="mb-4">
          <Collapse
            size="small"
            items={[
              {
                key: "detail",
                label: `持仓估值明细（${valuation.holdings.length} 个标的）`,
                children: <ValuationTable valuation={valuation} />,
              },
            ]}
          />
        </div>
      )}

      {/* Distribution charts */}
      <Row gutter={[16, 16]}>
        <Col xs={24} md={8}>
          <Card title="市场分布">
            <PieChart data={overview.market_distribution} height={260} currencyCode={baseCurrency} />
          </Card>
        </Col>
        <Col xs={24} md={8}>
          <Card title="类别分布">
            <PieChart data={overview.category_distribution} height={260} currencyCode={baseCurrency} />
          </Card>
        </Col>
        <Col xs={24} md={8}>
          <Card title="账户分布">
            <PieChart data={overview.account_distribution} height={260} currencyCode={baseCurrency} />
          </Card>
        </Col>
      </Row>

      {/* Stock distribution chart */}
      {overview.stock_distribution.length > 0 && (
        <Row gutter={[16, 16]} className="mt-4">
          <Col xs={24}>
            <Card title="个股分布">
              <PieChart data={overview.stock_distribution} height={360} currencyCode={baseCurrency} />
            </Card>
          </Col>
        </Row>
      )}

      {/* PnL charts */}
      {(gainersData.length > 0 || losersData.length > 0) && (
        <Row gutter={[16, 16]} className="mt-4">
          <Col xs={24} md={12}>
            <Card title="盈利 Top 5">
              <BarChart data={gainersData} colorByValue height={220} />
            </Card>
          </Col>
          <Col xs={24} md={12}>
            <Card title="亏损 Top 5">
              <BarChart data={losersData} colorByValue height={220} />
            </Card>
          </Col>
        </Row>
      )}
    </div>
  );
}
