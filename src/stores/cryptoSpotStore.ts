import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { CryptoSpot, CreateCryptoSpotPayload, UpdateCryptoSpotPayload } from "../types";

interface CryptoSpotState {
  cryptoSpots: CryptoSpot[];
  loading: boolean;
  error: string | null;
  fetchCryptoSpots: () => Promise<void>;
  createCryptoSpot: (payload: CreateCryptoSpotPayload) => Promise<CryptoSpot>;
  updateCryptoSpot: (payload: UpdateCryptoSpotPayload) => Promise<CryptoSpot>;
  deleteCryptoSpot: (id: string) => Promise<void>;
}

export const useCryptoSpotStore = create<CryptoSpotState>((set, _get) => ({
  cryptoSpots: [],
  loading: false,
  error: null,

  fetchCryptoSpots: async () => {
    set({ loading: true, error: null });
    try {
      const cryptoSpots = await invoke<CryptoSpot[]>("list_crypto_spots");
      set({ cryptoSpots, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  createCryptoSpot: async (payload) => {
    const cryptoSpot = await invoke<CryptoSpot>("create_crypto_spot", {
      symbol: payload.symbol,
      name: payload.name ?? null,
      buyPrice: payload.buy_price,
      shares: payload.shares,
      fee: payload.fee ?? null,
      exchange:  payload.exchange ?? null,
      notes: payload.notes ?? null,
    });
    set((state) => ({ cryptoSpots: [...state.cryptoSpots, cryptoSpot] }));
    return cryptoSpot;
  },

  updateCryptoSpot: async (payload) => {
    const cryptoSpot = await invoke<CryptoSpot>("update_crypto_spot", {
      id: payload.id,
      symbol: payload.symbol ?? null,
      name: payload.name ?? null,
      buyPrice: payload.buy_price ?? null,
      shares: payload.shares ?? null,
      fee: payload.fee ?? null,
      exchange:  payload.exchange ?? null,
      notes: payload.notes ?? null,
    });
    set((state) => ({
      cryptoSpots: state.cryptoSpots.map((c) => (c.id === cryptoSpot.id ? cryptoSpot : c)),
    }));
    return cryptoSpot;
  },

  deleteCryptoSpot: async (id) => {
    await invoke("delete_crypto_spot", { id });
    set((state) => ({
      cryptoSpots: state.cryptoSpots.filter((c) => c.id !== id),
    }));
  },
}));
