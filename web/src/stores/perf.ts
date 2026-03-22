import { create } from "zustand";

type PerfState = {
  lastTileMs: number;
  cacheHits: number;
  cacheMisses: number;
  recordTileLoad: (ms: number, fromCache: boolean) => void;
  hitRatio: () => number;
};

export const usePerfStore = create<PerfState>((set, get) => ({
  lastTileMs: 0,
  cacheHits: 0,
  cacheMisses: 0,
  recordTileLoad: (ms, fromCache) =>
    set((s) => ({
      lastTileMs: ms,
      cacheHits: fromCache ? s.cacheHits + 1 : s.cacheHits,
      cacheMisses: fromCache ? s.cacheMisses : s.cacheMisses + 1,
    })),
  hitRatio: () => {
    const { cacheHits, cacheMisses } = get();
    const total = cacheHits + cacheMisses;
    return total === 0 ? 0 : cacheHits / total;
  },
}));
