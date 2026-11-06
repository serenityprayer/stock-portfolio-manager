import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  AdvisorReply,
  ContractPriceInput,
  ContractRiskItem,
  DataQualityReport,
  DisciplineScore,
  DisciplineStats,
  DueReview,
  ExternalCashFlow,
  HwmState,
  InvestmentMemo,
  MemoComparison,
  MemoInput,
  RiskDashboard,
  RoundTrip,
  RoundTripStats,
  StressOverride,
  StressResult,
  TradeAnalysis,
  TradeVerdict,
  WatchItem,
  WatchItemInput,
} from "../types";

export interface NewCashFlow {
  accountId: string;
  currency: string;
  flowType: "DEPOSIT" | "WITHDRAW" | "DIVIDEND_ADJUST";
  amount: number;
  occurredAt: string;
  note?: string;
}

interface RiskState {
  dashboard: RiskDashboard | null;
  flows: ExternalCashFlow[];
  hwm: HwmState | null;
  watchlist: WatchItem[];
  stats: DisciplineStats | null;
  loading: boolean;
  error: string | null;

  // P2
  roundTrips: RoundTrip[];
  tripStats: RoundTripStats | null;
  dueReviews: DueReview[];
  disciplineScore: DisciplineScore | null;
  memos: InvestmentMemo[];
  memoComparison: MemoComparison | null;

  // P3
  analysis: TradeAnalysis | null;
  advisorReply: AdvisorReply | null;
  aiBusy: boolean;

  // P4
  stressResult: StressResult | null;
  contractRisks: ContractRiskItem[];
  quality: DataQualityReport | null;

  fetchDashboard: () => Promise<void>;
  evaluate: () => Promise<void>;
  acknowledge: (id: string) => Promise<void>;
  updateRule: (
    id: string,
    enabled: boolean | null,
    threshold: number | null
  ) => Promise<void>;
  fetchFlows: () => Promise<void>;
  fetchHwm: () => Promise<void>;
  recordFlow: (payload: NewCashFlow) => Promise<boolean>;
  deleteFlow: (id: string) => Promise<boolean>;
  recalibrate: (reason: string) => Promise<boolean>;
  fetchWatchlist: () => Promise<void>;
  addWatchItem: (input: WatchItemInput) => Promise<boolean>;
  removeWatchItem: (id: string) => Promise<boolean>;
  fetchStats: () => Promise<void>;
  precheck: (symbol: string, market: string, action: string) => Promise<TradeVerdict>;

  fetchRoundTrips: (symbol?: string | null) => Promise<void>;
  fetchDueReviews: (includeCompleted?: boolean) => Promise<void>;
  backfillCapsules: () => Promise<number>;
  completeReview: (id: string, quality: string, content: string) => Promise<boolean>;
  fetchDisciplineScore: () => Promise<void>;
  fetchMemos: () => Promise<void>;
  createMemo: (input: MemoInput) => Promise<boolean>;
  deleteMemo: (id: string) => Promise<boolean>;
  compareMemo: (id: string | null) => Promise<void>;

  analyzeTrade: (transactionId: string) => Promise<boolean>;
  askAdvisor: (message: string) => Promise<boolean>;

  stressTest: (overrides: StressOverride[]) => Promise<boolean>;
  fetchContractRisk: (prices: ContractPriceInput[]) => Promise<void>;
  fetchQuality: () => Promise<void>;
}

const errText = (err: unknown) => String(err);

export const useRiskStore = create<RiskState>((set, get) => ({
  dashboard: null,
  flows: [],
  hwm: null,
  watchlist: [],
  stats: null,
  loading: false,
  error: null,

  roundTrips: [],
  tripStats: null,
  dueReviews: [],
  disciplineScore: null,
  memos: [],
  memoComparison: null,
  analysis: null,
  advisorReply: null,
  aiBusy: false,
  stressResult: null,
  contractRisks: [],
  quality: null,

  fetchDashboard: async () => {
    set({ loading: true, error: null });
    try {
      const dashboard = await invoke<RiskDashboard>("get_risk_dashboard");
      set({ dashboard, loading: false });
    } catch (err) {
      set({ error: errText(err), loading: false });
    }
  },

  evaluate: async () => {
    set({ loading: true, error: null });
    try {
      await invoke("evaluate_risk");
      await get().fetchDashboard();
    } catch (err) {
      set({ error: errText(err), loading: false });
    }
  },

  acknowledge: async (id) => {
    try {
      await invoke("acknowledge_risk_event", { id });
      await get().fetchDashboard();
    } catch (err) {
      set({ error: errText(err) });
    }
  },

  updateRule: async (id, enabled, threshold) => {
    try {
      await invoke("update_risk_rule", { id, enabled, threshold });
      await get().fetchDashboard();
    } catch (err) {
      set({ error: errText(err) });
    }
  },

  fetchFlows: async () => {
    try {
      const flows = await invoke<ExternalCashFlow[]>("list_external_flows");
      set({ flows });
    } catch (err) {
      set({ error: errText(err) });
    }
  },

  fetchHwm: async () => {
    try {
      const hwm = await invoke<HwmState>("get_hwm_state");
      set({ hwm });
    } catch (err) {
      set({ error: errText(err) });
    }
  },

  recordFlow: async (payload) => {
    try {
      await invoke("record_external_flow", {
        accountId: payload.accountId,
        currency: payload.currency,
        flowType: payload.flowType,
        amount: payload.amount,
        occurredAt: payload.occurredAt,
        note: payload.note ?? null,
      });
      await Promise.all([get().fetchFlows(), get().fetchDashboard(), get().fetchHwm()]);
      return true;
    } catch (err) {
      set({ error: errText(err) });
      return false;
    }
  },

  deleteFlow: async (id) => {
    try {
      await invoke("delete_external_flow", { id });
      await Promise.all([get().fetchFlows(), get().fetchDashboard(), get().fetchHwm()]);
      return true;
    } catch (err) {
      set({ error: errText(err) });
      return false;
    }
  },

  recalibrate: async (reason) => {
    try {
      await invoke("recalibrate_hwm", { reason });
      await Promise.all([get().fetchHwm(), get().fetchDashboard()]);
      return true;
    } catch (err) {
      set({ error: errText(err) });
      return false;
    }
  },

  fetchWatchlist: async () => {
    try {
      const watchlist = await invoke<WatchItem[]>("get_watchlist");
      set({ watchlist });
    } catch (err) {
      set({ error: errText(err) });
    }
  },

  addWatchItem: async (input) => {
    try {
      await invoke("add_watchlist_item", { input });
      await Promise.all([get().fetchWatchlist(), get().fetchStats()]);
      return true;
    } catch (err) {
      set({ error: errText(err) });
      return false;
    }
  },

  removeWatchItem: async (id) => {
    try {
      await invoke("remove_watchlist_item", { id });
      await Promise.all([get().fetchWatchlist(), get().fetchStats()]);
      return true;
    } catch (err) {
      set({ error: errText(err) });
      return false;
    }
  },

  fetchStats: async () => {
    try {
      const stats = await invoke<DisciplineStats>("get_discipline_stats");
      set({ stats });
    } catch (err) {
      set({ error: errText(err) });
    }
  },

  precheck: async (symbol, market, action) =>
    invoke<TradeVerdict>("precheck_trade", { symbol, market, action }),

  // ── P2 ──────────────────────────────────────────────────────
  fetchRoundTrips: async (symbol) => {
    try {
      const [roundTrips, tripStats] = await Promise.all([
        invoke<RoundTrip[]>("get_round_trips", { symbol: symbol ?? null }),
        invoke<RoundTripStats>("get_round_trip_stats", { symbol: symbol ?? null }),
      ]);
      set({ roundTrips, tripStats });
    } catch (err) {
      set({ error: errText(err) });
    }
  },

  fetchDueReviews: async (includeCompleted = false) => {
    try {
      const dueReviews = await invoke<DueReview[]>("list_due_reviews", {
        includeCompleted,
      });
      set({ dueReviews });
    } catch (err) {
      set({ error: errText(err) });
    }
  },

  backfillCapsules: async () => {
    try {
      const n = await invoke<number>("backfill_time_capsules");
      await Promise.all([get().fetchDueReviews(), get().fetchDisciplineScore()]);
      return n;
    } catch (err) {
      set({ error: errText(err) });
      return -1;
    }
  },

  completeReview: async (id, quality, content) => {
    try {
      await invoke("complete_review", {
        input: { id, quality, content },
      });
      await Promise.all([get().fetchDueReviews(), get().fetchDisciplineScore()]);
      return true;
    } catch (err) {
      set({ error: errText(err) });
      return false;
    }
  },

  fetchDisciplineScore: async () => {
    try {
      const disciplineScore = await invoke<DisciplineScore>("get_discipline_score");
      set({ disciplineScore });
    } catch (err) {
      set({ error: errText(err) });
    }
  },

  fetchMemos: async () => {
    try {
      const memos = await invoke<InvestmentMemo[]>("list_memos");
      set({ memos });
    } catch (err) {
      set({ error: errText(err) });
    }
  },

  createMemo: async (input) => {
    try {
      await invoke("create_memo", { input });
      await get().fetchMemos();
      return true;
    } catch (err) {
      set({ error: errText(err) });
      return false;
    }
  },

  deleteMemo: async (id) => {
    try {
      await invoke("delete_memo", { id });
      await get().fetchMemos();
      set({ memoComparison: null });
      return true;
    } catch (err) {
      set({ error: errText(err) });
      return false;
    }
  },

  compareMemo: async (id) => {
    if (!id) {
      set({ memoComparison: null });
      return;
    }
    try {
      const memoComparison = await invoke<MemoComparison>("compare_memo", { id });
      set({ memoComparison });
    } catch (err) {
      set({ error: errText(err) });
    }
  },

  // ── P3 ──────────────────────────────────────────────────────
  analyzeTrade: async (transactionId) => {
    set({ aiBusy: true, error: null, analysis: null });
    try {
      const analysis = await invoke<TradeAnalysis>("analyze_trade", { transactionId });
      set({ analysis, aiBusy: false });
      return true;
    } catch (err) {
      set({ error: errText(err), aiBusy: false });
      return false;
    }
  },

  askAdvisor: async (message) => {
    set({ aiBusy: true, error: null, advisorReply: null });
    try {
      const advisorReply = await invoke<AdvisorReply>("chat_with_advisor", { message });
      set({ advisorReply, aiBusy: false });
      return true;
    } catch (err) {
      set({ error: errText(err), aiBusy: false });
      return false;
    }
  },

  // ── P4 ──────────────────────────────────────────────────────
  stressTest: async (overrides) => {
    try {
      const stressResult = await invoke<StressResult>("stress_test", { overrides });
      set({ stressResult });
      return true;
    } catch (err) {
      set({ error: errText(err) });
      return false;
    }
  },

  fetchContractRisk: async (prices) => {
    try {
      const contractRisks = await invoke<ContractRiskItem[]>("evaluate_contract_risk", {
        prices,
      });
      set({ contractRisks });
    } catch (err) {
      set({ error: errText(err) });
    }
  },

  fetchQuality: async () => {
    try {
      const quality = await invoke<DataQualityReport>("check_data_quality");
      set({ quality });
    } catch (err) {
      set({ error: errText(err) });
    }
  },
}));
