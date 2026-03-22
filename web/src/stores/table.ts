import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import { enableMapSet, castDraft } from "immer";
import type { Table } from "apache-arrow";
import type { CellRange, SourceMeta, Viewport } from "../lib/types";
import { tileKey } from "../lib/viewport";
import { ClientIndex } from "../lib/client-index";

enableMapSet();

const TILE_STORE_MAX_BYTES = 256 * 1024 * 1024;

function tableByteSize(table: Table): number {
  let total = 0;
  for (const batch of table.batches) {
    const bl = (batch as unknown as { byteLength?: number }).byteLength;
    if (typeof bl === "number") {
      total += bl;
    } else {
      for (let i = 0; i < batch.numCols; i++) {
        const col = batch.getChildAt(i);
        if (col) {
          for (let chunk = 0; chunk < col.data.length; chunk++) {
            const d = col.data[chunk];
            if (d) {
              const buffers = d.buffers as unknown as (ArrayBuffer | null)[];
              for (const buf of buffers) {
                if (buf) total += buf.byteLength;
              }
            }
          }
        }
      }
      total = Math.ceil(total * 1.3);
    }
  }
  return total || 1024;
}

type TileMap = Map<string, Table>;

function posKey(row: number, col: number): string {
  return `${row}:${col}`;
}

type TableState = {
  source: SourceMeta | null;
  viewport: Viewport;
  selection: CellRange | null;
  tiles: TileMap;
  tileBytes: Map<string, number>;
  currentBytes: number;
  loading: Set<string>;
  provisionalTiles: Set<string>;
  staleTiles: Map<string, Table>;
  clientIndex: ClientIndex | null;

  setSource: (source: SourceMeta | null) => void;
  setViewport: (viewport: Partial<Viewport>) => void;
  setSelection: (selection: CellRange | null) => void;
  cacheTile: (row: number, col: number, table: Table, viewHash: string, isProvisional?: boolean) => void;
  markLoading: (row: number, col: number, viewHash: string) => void;
  unmarkLoading: (row: number, col: number, viewHash: string) => void;
  hasTile: (row: number, col: number, viewHash: string) => boolean;
  isLoading: (row: number, col: number, viewHash: string) => boolean;
  invalidateTiles: () => void;
  stashTilesAsStale: () => void;
  clearStaleTiles: () => void;
  setClientIndex: (index: ClientIndex | null) => void;
};

export const useTableStore = create<TableState>()(
  immer((set, get) => ({
    source: null,
    viewport: { scrollX: 0, scrollY: 0, width: 0, height: 0 },
    selection: null,
    tiles: new Map(),
    tileBytes: new Map(),
    currentBytes: 0,
    loading: new Set(),
    provisionalTiles: new Set(),
    staleTiles: new Map(),
    clientIndex: null,

    setSource: (source) =>
      set({ source, tiles: new Map(), tileBytes: new Map(), currentBytes: 0, loading: new Set(), provisionalTiles: new Set(), staleTiles: new Map(), clientIndex: null }),

    setViewport: (partial) =>
      set((s) => {
        s.viewport = { ...s.viewport, ...partial };
      }),

    setSelection: (selection) => set({ selection }),

    cacheTile: (row, col, table, viewHash, isProvisional = false) =>
      set((s) => {
        const key = tileKey(row, col, viewHash);
        const existingBytes = s.tileBytes.get(key) ?? 0;
        if (existingBytes > 0) s.currentBytes -= existingBytes;
        s.tiles.delete(key);
        const bytes = tableByteSize(table);
        s.tiles.set(key, castDraft(table));
        s.tileBytes.set(key, bytes);
        s.currentBytes += bytes;
        s.loading.delete(key);
        if (isProvisional) {
          s.provisionalTiles.add(key);
        } else {
          s.provisionalTiles.delete(key);
        }
        s.staleTiles.delete(posKey(row, col));
        while (s.currentBytes > TILE_STORE_MAX_BYTES && s.tiles.size > 0) {
          const oldest = s.tiles.keys().next().value;
          if (oldest) {
            s.currentBytes -= s.tileBytes.get(oldest) ?? 0;
            s.tiles.delete(oldest);
            s.tileBytes.delete(oldest);
            s.provisionalTiles.delete(oldest);
          }
        }
      }),

    markLoading: (row, col, viewHash) =>
      set((s) => {
        const key = tileKey(row, col, viewHash);
        s.loading.add(key);
      }),

    unmarkLoading: (row, col, viewHash) =>
      set((s) => {
        const key = tileKey(row, col, viewHash);
        s.loading.delete(key);
      }),

    hasTile: (row, col, viewHash) => get().tiles.has(tileKey(row, col, viewHash)),

    isLoading: (row, col, viewHash) => get().loading.has(tileKey(row, col, viewHash)),

    setClientIndex: (index) => set({ clientIndex: index }),

    invalidateTiles: () =>
      set((s) => {
        s.tiles.clear();
        s.tileBytes.clear();
        s.currentBytes = 0;
        s.loading.clear();
        s.provisionalTiles.clear();
        s.staleTiles.clear();
        s.clientIndex = null;
      }),

    stashTilesAsStale: () =>
      set((s) => {
        s.staleTiles.clear();
        s.tiles.forEach((table, key) => {
          const colonIdx = key.indexOf(":");
          if (colonIdx >= 0) {
            const pk = key.slice(colonIdx + 1);
            s.staleTiles.set(pk, castDraft(table as unknown as Table));
          }
        });
        s.tiles.clear();
        s.tileBytes.clear();
        s.currentBytes = 0;
        s.loading.clear();
        s.provisionalTiles.clear();
        s.clientIndex = null;
      }),

    clearStaleTiles: () =>
      set((s) => {
        s.staleTiles.clear();
      }),
  }))
);
