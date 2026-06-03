import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { CryptoContract, CreateCryptoContractPayload, UpdateCryptoContractPayload } from "../types";

interface CryptoContractState {
  contracts: CryptoContract[];
  loading: boolean;
  error: string | null;
  fetchContracts: () => Promise<void>;
  createContract: (payload: CreateCryptoContractPayload) => Promise<CryptoContract>;
  updateContract: (payload: UpdateCryptoContractPayload) => Promise<CryptoContract>;
  deleteContract: (id: string) => Promise<void>;
  fetchQuotes: () => Promise<Record<string, { price: number; change: number; changePercent: number }>>;
}

export const useCryptoContractStore = create<CryptoContractState>((set, get) => ({
  contracts: [],
  loading: false,
  error: null,

  fetchContracts: async () => {
    set({ loading: true, error: null });
    try {
      const contracts = await invoke<CryptoContract[]>("list_crypto_contracts");
      set({ contracts, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  createContract: async (payload) => {
    const contract = await invoke<CryptoContract>("create_crypto_contract", {
      symbol: payload.symbol,
      name: payload.name ?? null,
      assetType: payload.asset_type ?? "crypto",
      positionType: payload.position_type,
      openPrice: payload.open_price,
      shares: payload.shares,
      leverage: payload.leverage,
      fee: payload.fee ?? null,
      exchange: payload.exchange ?? null,
      notes: payload.notes ?? null,
    });
    set((state) => ({ contracts: [...state.contracts, contract] }));
    return contract;
  },

  updateContract: async (payload) => {
    const contract = await invoke<CryptoContract>("update_crypto_contract", {
      id: payload.id,
      symbol: payload.symbol ?? null,
      name: payload.name ?? null,
      assetType: payload.asset_type ?? null,
      positionType: payload.position_type ?? null,
      openPrice: payload.open_price ?? null,
      shares: payload.shares ?? null,
      leverage: payload.leverage ?? null,
      fee: payload.fee ?? null,
      exchange: payload.exchange ?? null,
      notes: payload.notes ?? null,
    });
    set((state) => ({
      contracts: state.contracts.map((c) => (c.id === contract.id ? contract : c)),
    }));
    return contract;
  },

  deleteContract: async (id) => {
    await invoke("delete_crypto_contract", { id });
    set((state) => ({
      contracts: state.contracts.filter((c) => c.id !== id),
    }));
  },

  fetchQuotes: async () => {
    const contracts = get().contracts;
    if (contracts.length === 0) return {};

    const cryptoSymbols = contracts
      .filter((c) => c.asset_type === "crypto")
      .map((c) => c.symbol)
      .join(",");
    const tradfiSymbols = contracts
      .filter((c) => c.asset_type === "tradfi")
      .map((c) => c.symbol)
      .join(",");

    const [cryptoResult, tradfiResult] = await Promise.all([
      cryptoSymbols
        ? invoke<Record<string, { price: number; change: number; changePercent: number }>>(
            "fetch_crypto_quotes",
            { symbols: cryptoSymbols },
          )
        : Promise.resolve({}),
      tradfiSymbols
        ? invoke<Record<string, { price: number; change: number; changePercent: number }>>(
            "fetch_tradfi_quotes",
            { symbols: tradfiSymbols },
          )
        : Promise.resolve({}),
    ]);

    return { ...cryptoResult, ...tradfiResult };
  },
}));
