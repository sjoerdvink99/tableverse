import { create } from "zustand";
import type { AggExpr, Predicate, SortKey, ViewExpr, ViewOp } from "../lib/viewExpr";
import { computeViewHash } from "../lib/viewHash";
import { addPredicate, removePredicate } from "../lib/addPredicate";
import type { ColumnInfo } from "../lib/types";

export type ColumnView = {
  name: string;
  visible: boolean;
  pinned: "left" | "right" | null;
  displayIndex: number;
};

type ViewState = {
  sourceId: string | null;
  ops: ViewOp[];
  viewHash: string;
  virtualRowCount: number | null;
  virtualSchema: ColumnInfo[] | null;
  columnViews: ColumnView[];

  setSourceId: (id: string | null) => void;
  setOps: (ops: ViewOp[]) => void;
  addOp: (op: ViewOp) => void;
  removeOp: (index: number) => void;
  updateOp: (index: number, op: ViewOp) => void;
  clearOps: () => void;

  addPredicate: (pred: Predicate) => void;
  removePredicate: (column: string) => void;
  setSort: (keys: SortKey[]) => void;
  clearSort: () => void;
  setGroupBy: (keys: string[], aggs: AggExpr[]) => void;
  clearGroupBy: () => void;

  setRename: (mappings: [string, string][]) => void;
  clearRename: () => void;
  setLimit: (n: number) => void;
  clearLimit: () => void;
  setSearch: (query: string, textColumns: string[]) => void;
  clearSearch: () => void;

  setVirtualRowCount: (count: number | null) => void;
  setVirtualSchema: (schema: ColumnInfo[] | null) => void;

  initColumnViews: (columns: ColumnInfo[]) => void;
  toggleColumnVisibility: (name: string) => void;
  setColumnPinned: (name: string, side: "left" | "right" | null) => void;
  reorderColumn: (from: number, to: number) => void;
  visibleColumns: () => ColumnView[];

  buildViewExpr: () => ViewExpr | null;
};

export const useViewStore = create<ViewState>((set, get) => ({
  sourceId: null,
  ops: [],
  viewHash: "",
  virtualRowCount: null,
  virtualSchema: null,
  columnViews: [],

  setSourceId: (sourceId) =>
    set({ sourceId, ops: [], viewHash: computeViewHash([], sourceId), virtualRowCount: null, virtualSchema: null, columnViews: [] }),

  setOps: (ops) =>
    set((s) => ({ ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null })),

  addOp: (op) =>
    set((s) => {
      const ops = [...s.ops, op];
      return { ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null };
    }),

  removeOp: (index) =>
    set((s) => {
      const ops = s.ops.filter((_, i) => i !== index);
      return { ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null };
    }),

  updateOp: (index, op) =>
    set((s) => {
      const ops = s.ops.map((existing, i) => (i === index ? op : existing));
      return { ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null };
    }),

  clearOps: () =>
    set((s) => ({ ops: [], viewHash: computeViewHash([], s.sourceId), virtualRowCount: null })),

  addPredicate: (pred) =>
    set((s) => {
      const ops = addPredicate(s.ops, pred);
      return { ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null };
    }),

  removePredicate: (column) =>
    set((s) => {
      const ops = removePredicate(s.ops, column);
      return { ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null };
    }),

  setSort: (keys) =>
    set((s) => {
      const withoutSort = s.ops.filter((op) => op.type !== "sort");
      const ops = keys.length > 0 ? [...withoutSort, { type: "sort" as const, keys }] : withoutSort;
      return { ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null };
    }),

  clearSort: () =>
    set((s) => {
      const ops = s.ops.filter((op) => op.type !== "sort");
      return { ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null };
    }),

  setGroupBy: (keys, aggs) =>
    set((s) => {
      const withoutGroup = s.ops.filter((op) => op.type !== "group_by");
      const ops = [...withoutGroup, { type: "group_by" as const, keys, aggs }];
      return { ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null };
    }),

  clearGroupBy: () =>
    set((s) => {
      const ops = s.ops.filter((op) => op.type !== "group_by");
      return { ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null };
    }),

  setRename: (mappings) =>
    set((s) => {
      const withoutRename = s.ops.filter((op) => op.type !== "rename");
      const ops = [...withoutRename, { type: "rename" as const, mappings }];
      return { ops, viewHash: computeViewHash(ops, s.sourceId) };
    }),

  clearRename: () =>
    set((s) => {
      const ops = s.ops.filter((op) => op.type !== "rename");
      return { ops, viewHash: computeViewHash(ops, s.sourceId) };
    }),

  setLimit: (n) =>
    set((s) => {
      const withoutLimit = s.ops.filter((op) => op.type !== "limit");
      const ops = [...withoutLimit, { type: "limit" as const, n }];
      return { ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null };
    }),

  clearLimit: () =>
    set((s) => {
      const ops = s.ops.filter((op) => op.type !== "limit");
      return { ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null };
    }),

  setSearch: (query, textColumns) =>
    set((s) => {
      const withoutSearch = s.ops.filter((op) => {
        if (op.type !== "filter") return true;
        const p = op.predicate;
        if (p.op === "contains") return false;
        if (p.op === "or" && p.exprs.every((e) => e.op === "contains")) return false;
        return true;
      });
      if (!query || textColumns.length === 0) {
        const ops = withoutSearch;
        return { ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null };
      }
      const predicate: Predicate = textColumns.length === 1
        ? { op: "contains" as const, column: textColumns[0]!, value: query }
        : { op: "or" as const, exprs: textColumns.map(col => ({ op: "contains" as const, column: col, value: query })) };
      const ops = [...withoutSearch, { type: "filter" as const, predicate }];
      return { ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null };
    }),

  clearSearch: () =>
    set((s) => {
      const ops = s.ops.filter((op) => {
        if (op.type !== "filter") return true;
        const p = op.predicate;
        if (p.op === "contains") return false;
        if (p.op === "or" && p.exprs.every((e) => e.op === "contains")) return false;
        return true;
      });
      return { ops, viewHash: computeViewHash(ops, s.sourceId), virtualRowCount: null };
    }),

  setVirtualRowCount: (virtualRowCount) => set({ virtualRowCount }),
  setVirtualSchema: (virtualSchema) => set({ virtualSchema }),

  initColumnViews: (columns) =>
    set({
      columnViews: columns.map((col, i) => ({
        name: col.name,
        visible: true,
        pinned: null,
        displayIndex: i,
      })),
    }),

  toggleColumnVisibility: (name) =>
    set((s) => ({
      columnViews: s.columnViews.map((cv) =>
        cv.name === name ? { ...cv, visible: !cv.visible } : cv
      ),
    })),

  setColumnPinned: (name, side) =>
    set((s) => ({
      columnViews: s.columnViews.map((cv) =>
        cv.name === name ? { ...cv, pinned: side } : cv
      ),
    })),

  reorderColumn: (from, to) =>
    set((s) => {
      const sorted = [...s.columnViews].sort((a, b) => a.displayIndex - b.displayIndex);
      const spliced = sorted.splice(from, 1);
      const moved = spliced[0];
      if (!moved) return {};
      sorted.splice(to, 0, moved);
      return {
        columnViews: sorted.map((cv, i) => ({ ...cv, displayIndex: i })),
      };
    }),

  visibleColumns: () => {
    const { columnViews } = get();
    return columnViews
      .filter((cv) => cv.visible)
      .sort((a, b) => a.displayIndex - b.displayIndex);
  },

  buildViewExpr: () => {
    const { sourceId, ops } = get();
    if (!sourceId) return null;
    return { source_id: sourceId, ops };
  },
}));
