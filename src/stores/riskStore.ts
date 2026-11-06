import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { ExternalCashFlow, HwmState, RiskDashboard } from "../types";

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
  loading: boolean;
  error: string | null;

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
}

export const useRiskStore = create<RiskState>((set, get) => ({
  dashboard: null,
  flows: [],
  hwm: null,
  loading: false,
  error: null,

  fetchDashboard: async () => {
    set({ loading: true, error: null });
    try {
      const dashboard = await invoke<RiskDashboard>("get_risk_dashboard");
      set({ dashboard, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  evaluate: async () => {
    set({ loading: true, error: null });
    try {
      await invoke("evaluate_risk");
      await get().fetchDashboard();
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  acknowledge: async (id) => {
    try {
      await invoke("acknowledge_risk_event", { id });
      await get().fetchDashboard();
    } catch (err) {
      set({ error: String(err) });
    }
  },

  updateRule: async (id, enabled, threshold) => {
    try {
      await invoke("update_risk_rule", { id, enabled, threshold });
      await get().fetchDashboard();
    } catch (err) {
      set({ error: String(err) });
    }
  },

  fetchFlows: async () => {
    try {
      const flows = await invoke<ExternalCashFlow[]>("list_external_flows");
      set({ flows });
    } catch (err) {
      set({ error: String(err) });
    }
  },

  fetchHwm: async () => {
    try {
      const hwm = await invoke<HwmState>("get_hwm_state");
      set({ hwm });
    } catch (err) {
      set({ error: String(err) });
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
      set({ error: String(err) });
      return false;
    }
  },

  deleteFlow: async (id) => {
    try {
      await invoke("delete_external_flow", { id });
      await Promise.all([get().fetchFlows(), get().fetchDashboard(), get().fetchHwm()]);
      return true;
    } catch (err) {
      set({ error: String(err) });
      return false;
    }
  },

  recalibrate: async (reason) => {
    try {
      await invoke("recalibrate_hwm", { reason });
      await Promise.all([get().fetchHwm(), get().fetchDashboard()]);
      return true;
    } catch (err) {
      set({ error: String(err) });
      return false;
    }
  },
}));
