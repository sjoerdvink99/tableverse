import { useEffect, useMemo, useRef, useState } from "react";
import type { Table } from "apache-arrow";
import { useTableStore } from "../stores/table";
import { useViewStore } from "../stores/view";
import { useUiStore } from "../stores/ui";
import { useLandmarkStore } from "../stores/landmarkStore";
import { TileManager } from "../lib/tile-manager";
import { PrefetchModel } from "../lib/prefetch-model";
import { ClientIndex, CLIENT_INDEX_MAX_ROWS } from "../lib/client-index";
import {
  visibleTiles,
  viewportCenterTile,
  tileKey,
  tileRowsForZoom,
  tileColsForZoom,
  DEFAULT_CELL_H,
  DEFAULT_CELL_W,
  TILE_ROWS,
  TILE_COLS,
} from "../lib/viewport";

const JUMP_THRESHOLD_VIEWPORTS = 3;
const PREFETCH_SETTLE_MS = 60;
const SCROLL_HISTORY_SIZE = 5;
const AGG_THRESHOLD_ZOOM = 0.3;

type ScrollPoint = { t: number; y: number; x: number };
type ExplorationPhase = "foraging" | "sensemaking" | "navigation";

function classifyPhase(
  history: Array<{ t: number; y: number; x: number }>,
  zoom: number,
  prevZoom: number,
  lastSignificantMoveMs: number
): ExplorationPhase {
  if (Math.abs(zoom - prevZoom) > 0.15) return "navigation";
  if (history.length < 2) return "sensemaking";
  const speeds = history.slice(1).map((p, i) => {
    const prev = history[i]!;
    const dt = (p.t - prev.t) / 1000;
    if (dt <= 0) return 0;
    return Math.sqrt((p.y - prev.y) ** 2 + (p.x - prev.x) ** 2) / dt;
  });
  const avgSpeed = speeds.reduce((a, b) => a + b, 0) / speeds.length;
  const dwellMs = Date.now() - lastSignificantMoveMs;
  if (avgSpeed > 2000 || dwellMs < 1500) return "foraging";
  return "sensemaking";
}

const manager = new TileManager(256 * 1024 * 1024);
const prefetchModel = new PrefetchModel();

export function useTiles() {
  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);
  const cacheTile = useTableStore((s) => s.cacheTile);
  const markLoading = useTableStore((s) => s.markLoading);
  const unmarkLoading = useTableStore((s) => s.unmarkLoading);
  const hasTile = useTableStore((s) => s.hasTile);
  const isLoading = useTableStore((s) => s.isLoading);
  const invalidateTiles = useTableStore((s) => s.invalidateTiles);
  const stashTilesAsStale = useTableStore((s) => s.stashTilesAsStale);
  const clearStaleTiles = useTableStore((s) => s.clearStaleTiles);

  const sourceId = useViewStore((s) => s.sourceId);
  const ops = useViewStore((s) => s.ops);
  const viewHash = useViewStore((s) => s.viewHash);
  const virtualRowCount = useViewStore((s) => s.virtualRowCount);
  const zoom = useUiStore((s) => s.zoom);
  const activeJobId = useUiStore((s) => s.activeJobId);
  const setActiveJobId = useUiStore((s) => s.setActiveJobId);

  const viewExpr = useMemo(
    () => (sourceId ? { source_id: sourceId, ops } : null),
    [sourceId, ops]
  );

  const landmarks = useLandmarkStore((s) => s.landmarks);

  const clientIndex = useTableStore((s) => s.clientIndex);
  const setClientIndex = useTableStore((s) => s.setClientIndex);

  const [settleEpoch, setSettleEpoch] = useState(0);

  const prevViewHash = useRef("");
  const prevScrollY = useRef(0);
  const prevTileRows = useRef(256);
  const prefetchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const scrollHistory = useRef<ScrollPoint[]>([]);
  const prevZoomRef = useRef(zoom);
  const lastSignificantMoveRef = useRef(Date.now());
  const pendingIndexTablesRef = useRef<Map<string, Table>>(new Map());
  const lastScrollRef = useRef({ x: 0, y: 0, t: 0 });
  const velocityRef = useRef({ vy: 0, vx: 0 });

  useEffect(() => {
    if (prevViewHash.current !== viewHash) {
      manager.invalidate();
      prefetchModel.reset();
      stashTilesAsStale();
      pendingIndexTablesRef.current.clear();
      prevViewHash.current = viewHash;
    }
  }, [viewHash, stashTilesAsStale]);

  useEffect(() => {
    if (!activeJobId) return;
    const BASE = (import.meta.env.VITE_API_BASE as string | undefined) ?? "";
    const es = new EventSource(`${BASE}/api/v1/jobs/${activeJobId}/stream`);
    const onComplete = () => {
      setActiveJobId(null);
      stashTilesAsStale();
      manager.invalidate();
      prefetchModel.reset();
      setSettleEpoch((e) => e + 1);
      es.close();
    };
    es.addEventListener("job_complete", onComplete);
    es.addEventListener("job_failed", () => {
      setActiveJobId(null);
      clearStaleTiles();
      es.close();
    });
    es.onerror = () => es.close();
    return () => es.close();
  }, [activeJobId, setActiveJobId, stashTilesAsStale, clearStaleTiles]);

  useEffect(() => {
    if (!source || !viewExpr) return;

    const cellW = DEFAULT_CELL_W * zoom;
    const cellH = DEFAULT_CELL_H * zoom;
    const tileRows = tileRowsForZoom(zoom);
    const tileCols = tileColsForZoom(zoom);

    if (tileRows !== prevTileRows.current) {
      manager.invalidate();
      invalidateTiles();
      pendingIndexTablesRef.current.clear();
      prevTileRows.current = tileRows;
    }

    const now = Date.now();
    const dt = Math.max(1, now - lastScrollRef.current.t);
    velocityRef.current.vy = Math.abs(viewport.scrollY - lastScrollRef.current.y) / dt;
    velocityRef.current.vx = Math.abs(viewport.scrollX - lastScrollRef.current.x) / dt;
    lastScrollRef.current = { x: viewport.scrollX, y: viewport.scrollY, t: now };

    const history = scrollHistory.current;
    history.push({ t: now, y: viewport.scrollY, x: viewport.scrollX });
    if (history.length > SCROLL_HISTORY_SIZE) history.shift();

    const prevX = history[history.length - 2]?.x ?? viewport.scrollX;
    if (
      Math.abs(viewport.scrollY - prevScrollY.current) > 50 ||
      Math.abs(viewport.scrollX - prevX) > 50
    ) {
      lastSignificantMoveRef.current = Date.now();
    }

    let vyTiles = 0;
    let vxTiles = 0;
    if (history.length >= 2) {
      const first = history[0]!;
      const last = history[history.length - 1]!;
      const dtTiles = (last.t - first.t) / 1000;
      if (dtTiles > 0) {
        vyTiles = (last.y - first.y) / dtTiles / cellH;
        vxTiles = (last.x - first.x) / dtTiles / cellW;
      }
    }
    manager.setViewportVelocity(vyTiles, vxTiles);

    const isJump =
      Math.abs(viewport.scrollY - prevScrollY.current) > viewport.height * JUMP_THRESHOLD_VIEWPORTS;
    prevScrollY.current = viewport.scrollY;

    const phase = classifyPhase(
      history,
      zoom,
      prevZoomRef.current,
      lastSignificantMoveRef.current
    );
    prevZoomRef.current = zoom;

    const isRapidPan = velocityRef.current.vy > 0.02 || velocityRef.current.vx > 0.02;

    const prefetch = isJump || isRapidPan ? 0 : phase === "sensemaking" ? 1 : 0;
    const coords = visibleTiles(viewport, cellW, cellH, zoom, prefetch);

    const center = viewportCenterTile(viewport, cellW, cellH, zoom);
    manager.setViewportCenter(center.row, center.col);

    if (landmarks.length > 0) {
      const landmarkTileRowNums = landmarks.map((l) => Math.floor(l.rowOffset / TILE_ROWS));
      manager.setLandmarkPriorities(landmarkTileRowNums);
    }

    const tileMode = zoom < AGG_THRESHOLD_ZOOM ? "agg" as const : undefined;

    const effectiveRowCount = virtualRowCount ?? source.n_rows;
    const visibleKeys = new Set<string>();
    for (const coord of coords) {
      const rowOffset = coord.row * tileRows;
      const colOffset = coord.col * tileCols;
      if (rowOffset < effectiveRowCount && colOffset < source.n_cols) {
        visibleKeys.add(tileKey(rowOffset, colOffset, viewHash));
      }
    }

    manager.cancelStale(visibleKeys);

    if (isJump) {
      if (prefetchTimerRef.current !== null) {
        clearTimeout(prefetchTimerRef.current);
      }
      prefetchTimerRef.current = setTimeout(() => {
        prefetchTimerRef.current = null;
        setSettleEpoch((e) => e + 1);
      }, PREFETCH_SETTLE_MS);
      return;
    }

    for (const coord of coords) {
      const rowOffset = coord.row * tileRows;
      const colOffset = coord.col * tileCols;

      if (rowOffset >= effectiveRowCount || colOffset >= source.n_cols) continue;
      if (hasTile(coord.row, coord.col, viewHash)) continue;
      if (isLoading(coord.row, coord.col, viewHash)) continue;
      if (manager.isFailed(rowOffset, colOffset, viewHash)) continue;

      prefetchModel.record(coord.row, coord.col);

      const isCached = manager.hasCached(rowOffset, colOffset, viewHash);
      if (!isCached) {
        markLoading(coord.row, coord.col, viewHash);
      }

      manager
        .getTile(
          {
            viewExpr,
            row: rowOffset,
            col: colOffset,
            rows: tileRows,
            cols: tileCols,
            mode: tileMode,
          },
          viewHash
        )
        .then((meta) => {
          if (useViewStore.getState().viewHash !== viewHash) return;
          cacheTile(coord.row, coord.col, meta.table, viewHash, meta.isProvisional);
          if (meta.isProvisional && meta.jobId) {
            setActiveJobId(meta.jobId);
          }
          if (
            !clientIndex &&
            ops.length === 0 &&
            source.n_rows < CLIENT_INDEX_MAX_ROWS &&
            !meta.isProvisional
          ) {
            const tKey = tileKey(rowOffset, colOffset, viewHash);
            pendingIndexTablesRef.current.set(tKey, meta.table);
            const totalTileRows = tileRowsForZoom(zoom);
            const totalTileCols = tileColsForZoom(zoom);
            const expectedTiles =
              Math.ceil(source.n_rows / totalTileRows) *
              Math.ceil(source.n_cols / totalTileCols);
            if (pendingIndexTablesRef.current.size >= expectedTiles) {
              const idx = new ClientIndex();
              idx.build(Array.from(pendingIndexTablesRef.current.values()));
              setClientIndex(idx);
              pendingIndexTablesRef.current.clear();
            }
          }
        })
        .catch((err) => {
          if (!isCached) unmarkLoading(coord.row, coord.col, viewHash);
          if (err instanceof DOMException && err.name === "AbortError") return;
          if (useViewStore.getState().viewHash !== viewHash) return;
          console.error(`Tile [${coord.row},${coord.col}] failed:`, err);
        });
    }

    if (!isRapidPan) {
      setTimeout(() => {
        const currentViewHash = useViewStore.getState().viewHash;
        if (currentViewHash !== viewHash) return;
        const predictions = prefetchModel.predict();
        const effectiveTileRows = tileRowsForZoom(zoom);
        const effectiveTileCols = tileColsForZoom(zoom);
        for (const { row, col } of predictions) {
          const rowOffset = row * effectiveTileRows;
          const colOffset = col * effectiveTileCols;
          if (rowOffset >= effectiveRowCount || colOffset >= source.n_cols) continue;
          if (!manager.hasCached(rowOffset, colOffset, viewHash) && !manager.isFailed(rowOffset, colOffset, viewHash)) {
            manager.getTile({ viewExpr, row: rowOffset, col: colOffset, rows: effectiveTileRows, cols: effectiveTileCols }, viewHash)
              .catch(() => {});
          }
        }
      }, 100);
    }

  }, [source, viewport, viewExpr, viewHash, virtualRowCount, zoom, ops, hasTile, isLoading, markLoading, unmarkLoading, cacheTile, setActiveJobId, invalidateTiles, clientIndex, setClientIndex, settleEpoch, landmarks]);

}
