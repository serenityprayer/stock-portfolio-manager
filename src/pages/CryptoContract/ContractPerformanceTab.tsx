import React, { useEffect, useMemo } from 'react';
import { Typography, Button, Spin, Alert, Row, Col, Select } from 'antd';
import { ReloadOutlined } from '@ant-design/icons';
import ReactECharts from 'echarts-for-react';
import { useContractPerformanceStore } from '../../stores/contractPerformanceStore';

const { Title, Text } = Typography;

const TIME_RANGES = [
  { label: '1个月', value: '1M' },
  { label: '3个月', value: '3M' },
  { label: '6个月', value: '6M' },
  { label: '年初至今', value: 'YTD' },
  { label: '全部', value: 'ALL' },
];

const ContractPerformanceTab: React.FC = () => {
  const summary = useContractPerformanceStore((s) => s.summary);
  const loading = useContractPerformanceStore((s) => s.loading);
  const error = useContractPerformanceStore((s) => s.error);
  const timeRange = useContractPerformanceStore((s) => s.timeRange);
  const fetchSummary = useContractPerformanceStore((s) => s.fetchSummary);
  const setTimeRange = useContractPerformanceStore((s) => s.setTimeRange);

  // useEffect 依赖数组完整，fetchSummary 是 Zustand 稳定引用
  useEffect(() => {
    fetchSummary();
  }, [timeRange, fetchSummary]);

  // ECharts 配置（双 Y 轴）—— 净值归一化（首日 = 1.0）
  const chartOption = useMemo(() => {
    if (!summary || summary.return_series.length === 0) return {};

    const firstNetValue = summary.return_series[0]?.net_value || 1;
    const dates = summary.return_series.map(p => p.date);
    const normalizedValues = summary.return_series.map(p => p.net_value / firstNetValue);
    const totalPnls = summary.return_series.map(p => p.total_pnl);

    return {
      tooltip: {
        trigger: 'axis',
        formatter: (params: any) => {
          const date = params[0].name;
          let html = `<div style="font-size:12px;"><strong>${date}</strong></div>`;
          params.forEach((p: any) => {
            const isRed = p.value >= 0;
            const color = isRed ? '#E53935' : '#43A047';
            const label = p.seriesName === '归一化净值' ? '归一化净值' : p.seriesName;
            const valStr = p.seriesName === '归一化净值'
              ? p.value.toFixed(4)
              : `${p.value >= 0 ? '+' : ''}${p.value.toFixed(2)}`;
            html += `<div style="font-size:12px;color:${color}">${label}: ${valStr}</div>`;
          });
          return html;
        }
      },
      legend: {
        data: ['归一化净值', '累计盈亏'],
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
          name: '归一化净值',
          position: 'left',
          axisLabel: {
            formatter: (val: number) => val.toFixed(2),
          },
        },
        {
          type: 'value',
          name: '盈亏 ($)',
          position: 'right',
          axisLabel: {
            formatter: (val: number) => `$${val.toFixed(0)}`,
          },
        },
      ],
      dataZoom: [
        { type: 'inside', start: 0, end: 100 },
        { type: 'slider', bottom: 30, start: 0, end: 100 },
      ],
      series: [
        {
          name: '归一化净值',
          type: 'line',
          data: normalizedValues,
          smooth: true,
          symbol: 'none',
          lineStyle: { width: 2 },
          areaStyle: { opacity: 0.06 },
        },
        {
          name: '累计盈亏',
          type: 'bar',
          yAxisIndex: 1,
          data: totalPnls,
          itemStyle: {
            color: (params: any) => params.value >= 0 ? '#E53935' : '#43A047',
          },
        },
      ],
    };
  }, [summary]);

  if (loading) {
    return <Spin size="large" style={{ display: 'block', margin: '100px auto' }} />;
  }

  if (error) {
    return <Alert type="error" message="加载失败" description={error} showIcon />;
  }

  return (
    <div>
      {/* Header */}
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 16 }}>
        <Title level={4} style={{ margin: 0 }}>合约收益曲线</Title>
        <div style={{ display: 'flex', gap: 8 }}>
          <Select
            value={timeRange}
            onChange={(val) => setTimeRange(val as any)}
            options={TIME_RANGES}
            style={{ width: 120 }}
          />
          <Button
            icon={<ReloadOutlined />}
            onClick={() => fetchSummary(true)}
            loading={loading}
          >
            刷新
          </Button>
        </div>
      </div>

      {/* 摘要卡片（6个） */}
      <Row gutter={16} style={{ marginBottom: 16 }}>
        <Col span={8}>
          <div style={{ background: '#fff', padding: 16, borderRadius: 8, boxShadow: '0 1px 4px rgba(0,0,0,0.1)' }}>
            <Text type="secondary">当前净值</Text>
            <div style={{ fontSize: 24, fontWeight: 'bold' }}>${summary?.current_net_value.toFixed(2) || '0.00'}</div>
          </div>
        </Col>
        <Col span={8}>
          <div style={{ background: '#fff', padding: 16, borderRadius: 8, boxShadow: '0 1px 4px rgba(0,0,0,0.1)' }}>
            <Text type="secondary">累计盈亏</Text>
            <div style={{ fontSize: 24, fontWeight: 'bold', color: (summary?.total_pnl || 0) >= 0 ? '#E53935' : '#43A047' }}>
              {(summary?.total_pnl || 0) >= 0 ? '+' : ''}{(summary?.total_pnl || 0).toFixed(2)}
            </div>
          </div>
        </Col>
        <Col span={8}>
          <div style={{ background: '#fff', padding: 16, borderRadius: 8, boxShadow: '0 1px 4px rgba(0,0,0,0.1)' }}>
            <Text type="secondary">总保证金</Text>
            <div style={{ fontSize: 24, fontWeight: 'bold' }}>${(summary?.total_margin || 0).toFixed(2)}</div>
          </div>
        </Col>
        <Col span={8}>
          <div style={{ background: '#fff', padding: 16, borderRadius: 8, boxShadow: '0 1px 4px rgba(0,0,0,0.1)' }}>
            <Text type="secondary">收益率</Text>
            <div style={{ fontSize: 24, fontWeight: 'bold', color: (summary?.total_return || 0) >= 0 ? '#E53935' : '#43A047' }}>
              {(summary?.total_return || 0) >= 0 ? '+' : ''}{(summary?.total_return || 0).toFixed(2)}%
            </div>
          </div>
        </Col>
        <Col span={8}>
          <div style={{ background: '#fff', padding: 16, borderRadius: 8, boxShadow: '0 1px 4px rgba(0,0,0,0.1)' }}>
            <Text type="secondary">最大回撤</Text>
            <div style={{ fontSize: 24, fontWeight: 'bold', color: '#FF9800' }}>
              -{(summary?.max_drawdown || 0).toFixed(2)}%
            </div>
          </div>
        </Col>
        <Col span={8}>
          <div style={{ background: '#fff', padding: 16, borderRadius: 8, boxShadow: '0 1px 4px rgba(0,0,0,0.1)' }}>
            <Text type="secondary">波动率</Text>
            <div style={{ fontSize: 24, fontWeight: 'bold', color: '#9C27B0' }}>
              {(summary?.volatility || 0).toFixed(2)}%
            </div>
          </div>
        </Col>
      </Row>

      {/* 图表 */}
      <div style={{ background: '#fff', padding: 16, borderRadius: 8, boxShadow: '0 1px 4px rgba(0,0,0,0.1)' }}>
        {summary && summary.return_series.length > 0 ? (
          <ReactECharts option={chartOption} style={{ height: 400 }} />
        ) : (
          <Alert type="info" message="暂无数据" description="请先添加合约持仓" showIcon />
        )}
      </div>
    </div>
  );
};

export default ContractPerformanceTab;
