import { create } from "zustand";
import type { ColumnStats, CorrelationMatrix, RowGroupStat } from "../lib/types";
import { fetchColumnStats } from "../lib/api";

type CachedCorrelations = {
  sourceId: string;
  matrix: CorrelationMatrix;
};

type StatsState = {
  stats: Map<string, ColumnStats>;
  pending: Set<string>;
  rowGroupStats: Map<string, RowGroupStat[]>;
  correlations: CachedCorrelations | null;
  fetchStats: (sourceId: string, colIndex: number) => void;
  setRowGroupStats: (key: string, stats: RowGroupStat[]) => void;
  setCorrelations: (cached: CachedCorrelations | null) => void;
  clearStats: () => void;
};

function statsKey(sourceId: string, colIndex: number): string {
  return `${sourceId}:${colIndex}`;
}

export const useStatsStore = create<StatsState>((set, get) => ({
  stats: new Map(),
  pending: new Set(),
  rowGroupStats: new Map(),
  correlations: null,

  setRowGroupStats: (key, stats) =>
    set((s) => {
      const next = new Map(s.rowGroupStats);
      next.set(key, stats);
      return { rowGroupStats: next };
    }),

  setCorrelations: (cached) => set({ correlations: cached }),

  fetchStats: (sourceId, colIndex) => {
    const key = statsKey(sourceId, colIndex);
    const { stats, pending } = get();
    if (stats.has(key) || pending.has(key)) return;

    set((s) => {
      const next = new Set(s.pending);
      next.add(key);
      return { pending: next };
    });

    fetchColumnStats(sourceId, colIndex)
      .then((result) => {
        set((s) => {
          const nextStats = new Map(s.stats);
          nextStats.set(key, result);
          const nextPending = new Set(s.pending);
          nextPending.delete(key);
          return { stats: nextStats, pending: nextPending };
        });
      })
      .catch(() => {
        set((s) => {
          const nextPending = new Set(s.pending);
          nextPending.delete(key);
          return { pending: nextPending };
        });
      });
  },

  clearStats: () =>
    set({ stats: new Map(), pending: new Set(), rowGroupStats: new Map(), correlations: null }),
}));

export function getStatsKey(sourceId: string, colIndex: number): string {
  return statsKey(sourceId, colIndex);
}
