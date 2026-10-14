import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { ContractPerformanceSummary, ContractReturnPoint } from '../types';

type TimeRange = '1M' | '3M' | '6M' | 'YTD' | 'ALL';

// ---------------------------------------------------------------------------
// Mock 数据开关（通过 Vite 环境变量控制）
// 在 .env.development 中设置 VITE_MOCK_CONTRACT=true 开启
// 生产环境永远不会启用
// ---------------------------------------------------------------------------
const MOCK_ENABLED = import.meta.env.VITE_MOCK_CONTRACT === 'true';

/**
 * 生成逼真的合约收益模拟数据
 * - 起始净值：10000
 * - 随机游走 + 趋势偏向（crypto 偏多，tradfi 偏空）
 * - 波动率：crypto 2%/天，tradfi 1%/天
 */
function generateMockSummary(timeRange: TimeRange): ContractPerformanceSummary {
  const end = new Date();
  let start = new Date();

  switch (timeRange) {
    case '1M': start.setMonth(end.getMonth() - 1); break;
    case '3M': start.setMonth(end.getMonth() - 3); break;
    case '6M': start.setMonth(end.getMonth() - 6); break;
    case 'YTD': start = new Date(end.getFullYear(), 0, 1); break;
    case 'ALL': start = new Date(end.getFullYear() - 1, 0, 1); break;
  }

  const dates: string[] = [];
  const cur = new Date(start);
  while (cur <= end) {
    // 跳过周末（简单模拟交易日）
    const dow = cur.getDay();
    if (dow !== 0 && dow !== 6) {
      dates.push(cur.toISOString().split('T')[0]);
    }
    cur.setDate(cur.getDate() + 1);
  }

  const return_series: ContractReturnPoint[] = [];
  let netValue = 10000.0;
  let totalMargin = 5000.0;
  const volatility = 0.02; // 2% 日波动
  let seed = 42;

  function seededRandom(): number {
    seed = (seed * 16807 + 0) % 2147483647;
    return (seed - 1) / 2147483646;
  }

  for (const date of dates) {
    // 随机日收益：-4% ~ +4%，略偏正（模拟长期持有合约的融资费损耗）
    const dailyReturn = (seededRandom() - 0.48) * 2 * volatility;
    netValue *= (1 + dailyReturn);

    const unrealizedPnl = netValue - totalMargin;
    const realizedPnl = 0.0; // 模拟数据暂不考虑已实现盈亏
    const totalPnl = unrealizedPnl + realizedPnl;
    const returnRate = totalMargin > 0 ? totalPnl / totalMargin * 100.0 : 0.0;

    return_series.push({
      date,
      net_value: Math.round(netValue * 100) / 100,
      total_margin: totalMargin,
      unrealized_pnl: Math.round(unrealizedPnl * 100) / 100,
      realized_pnl: realizedPnl,
      total_pnl: Math.round(totalPnl * 100) / 100,
      return_rate: Math.round(returnRate * 100) / 100,
    });
  }

  const lastPoint = return_series[return_series.length - 1];
  const totalPnl = lastPoint?.total_pnl ?? 0;
  const totalReturn = totalMargin > 0 ? totalPnl / totalMargin * 100.0 : 0.0;

  // 计算最大回撤
  let peak = return_series[0]?.net_value ?? netValue;
  let maxDrawdown = 0.0;
  for (const p of return_series) {
    if (p.net_value > peak) peak = p.net_value;
    const dd = (peak - p.net_value) / peak * 100.0;
    if (dd > maxDrawdown) maxDrawdown = dd;
  }

  // 计算波动率（年化）
  const dailyReturns: number[] = [];
  for (let i = 1; i < return_series.length; i++) {
    const prev = return_series[i - 1].net_value;
    const curr = return_series[i].net_value;
    if (prev > 0) dailyReturns.push((curr - prev) / prev);
  }
  const meanR = dailyReturns.reduce((a, b) => a + b, 0) / (dailyReturns.length || 1);
  const variance = dailyReturns.reduce((a, b) => a + (b - meanR) ** 2, 0) / (dailyReturns.length - 1 || 1);
  const annualizedVol = Math.sqrt(variance) * Math.sqrt(365) * 100;

  // 胜率
  const winDays = dailyReturns.filter(r => r > 0).length;
  const winRate = dailyReturns.length > 0 ? winDays / dailyReturns.length * 100 : 0;

  return {
    start_date: dates[0] ?? start.toISOString().split('T')[0],
    end_date: dates[dates.length - 1] ?? end.toISOString().split('T')[0],
    current_net_value: Math.round(lastPoint?.net_value ?? netValue * 100) / 100,
    total_margin: totalMargin,
    total_pnl: Math.round(totalPnl * 100) / 100,
    total_return: Math.round(totalReturn * 100) / 100,
    max_drawdown: Math.round(maxDrawdown * 100) / 100,
    volatility: Math.round(annualizedVol * 100) / 100,
    win_rate: Math.round(winRate * 100) / 100,
    return_series,
  };
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------
interface ContractPerformanceState {
  summary: ContractPerformanceSummary | null;
  loading: boolean;
  error: string | null;
  timeRange: TimeRange;
  earliestDate: string | null;

  fetchSummary: (forceRefresh?: boolean) => Promise<void>;
  initializeHistory: () => Promise<{ inserted: number; message: string }>;
  setTimeRange: (range: TimeRange) => void;
}

export const useContractPerformanceStore = create<ContractPerformanceState>((set, get) => ({
  summary: null,
  loading: false,
  error: null,
  timeRange: 'ALL',
  earliestDate: null,

  fetchSummary: async (forceRefresh = false) => {
    set({ loading: true, error: null });

    try {
      // ---------- Mock 模式 ----------
      if (MOCK_ENABLED) {
        // 模拟网络延迟
        await new Promise(r => setTimeout(r, 600));
        const summary = generateMockSummary(get().timeRange);
        set({ summary, loading: false });
        return;
      }

      // ---------- 真实后端调用 ----------
      const range = get().timeRange;
      const end = new Date();
      let start = new Date();

      switch (range) {
        case '1M': start.setMonth(end.getMonth() - 1); break;
        case '3M': start.setMonth(end.getMonth() - 3); break;
        case '6M': start.setMonth(end.getMonth() - 6); break;
        case 'YTD': start = new Date(end.getFullYear(), 0, 1); break;
        case 'ALL':
          let earliest = get().earliestDate;
          if (!earliest) {
            earliest = await invoke<string>('get_earliest_contract_date');
            set({ earliestDate: earliest });
          }
          start = new Date(earliest);
          break;
      }

      const startDate = start.toISOString().split('T')[0];
      const endDate = end.toISOString().split('T')[0];

      const summary = await invoke<ContractPerformanceSummary>(
        'get_contract_performance_summary',
        { startDate, endDate, forceRefresh }
      );

      set({ summary, loading: false });

    } catch (err) {
      console.error('[ContractPerformance] fetchSummary error:', err);
      set({ error: `获取合约收益数据失败: ${err}`, loading: false });
    }
  },

  initializeHistory: async () => {
    set({ loading: true, error: null });
    try {
      const result = await invoke<{ inserted: number; message: string }>(
        'initialize_contract_performance'
      );
      // 初始化后自动刷新
      await get().fetchSummary(true);
      return result;
    } catch (err) {
      console.error('[ContractPerformance] initializeHistory error:', err);
      set({ error: `初始化失败: ${err}`, loading: false });
      throw err;
    }
  },

  setTimeRange: (range) => {
    set({ timeRange: range });
  },
}));
