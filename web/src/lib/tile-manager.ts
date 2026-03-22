import type { Table } from "apache-arrow";
import { fetchViewTile, fetchViewTileBatch, type BatchTileRequestItem, type QueryTileParams, type TileMeta } from "./api";
import { TileCache } from "./tile-cache";
import { tileKey, TILE_ROWS, TILE_COLS } from "./viewport";
import { MinHeap } from "./min-heap";

const MAX_CONCURRENCY = 8;
const MAX_RETRIES = 3;
const FAILED_TTL_MS = 30_000;

type QueueItem = {
  key: string;
  tileRow: number;
  tileCol: number;
  params: QueryTileParams;
  resolve: (meta: TileMeta) => void;
  reject: (e: unknown) => void;
};

type InflightEntry = {
  promise: Promise<TileMeta>;
  reject: (e: unknown) => void;
  controller: AbortController | null;
  enqueuedAt: number;
};

export class TileManager {
  private cache: TileCache;
  private inflight = new Map<string, InflightEntry>();
  private failedAt = new Map<string, number>();
  private retries = new Map<string, number>();
  private queue = new MinHeap<QueueItem>();
  private active = 0;
  private generation = 0;
  private viewportCenterRow = 0;
  private viewportCenterCol = 0;
  private vy = 0;
  private vx = 0;
  private drainScheduled = false;
  private landmarkTileRows = new Set<number>();

  constructor(maxCacheBytes = 256 * 1024 * 1024) {
    this.cache = new TileCache(maxCacheBytes);
    this.startGC();
  }

  setViewportCenter(tileRow: number, tileCol: number): void {
    this.viewportCenterRow = tileRow;
    this.viewportCenterCol = tileCol;
  }

  setViewportVelocity(vy: number, vx: number): void {
    this.vy = vy;
    this.vx = vx;
  }

  setLandmarkPriorities(tileRows: number[]): void {
    this.landmarkTileRows = new Set(tileRows);
  }

  isFailed(rowOffset: number, colOffset: number, viewHash: string): boolean {
    const key = tileKey(rowOffset, colOffset, viewHash);
    const ts = this.failedAt.get(key);
    if (ts === undefined) return false;
    if (Date.now() - ts < FAILED_TTL_MS) return true;
    this.failedAt.delete(key);
    return false;
  }

  hasCached(row: number, col: number, viewHash: string): boolean {
    return this.cache.has(tileKey(row, col, viewHash));
  }

  cancelStale(visibleKeys: ReadonlySet<string>): void {
    const err = new DOMException("AbortError", "AbortError");
    for (const [key, entry] of this.inflight) {
      if (!visibleKeys.has(key)) {
        if (entry.controller !== null) {
          entry.controller.abort();
        }
        entry.reject(err);
        this.inflight.delete(key);
      }
    }
    const surviving: QueueItem[] = [];
    while (this.queue.size > 0) {
      const item = this.queue.pop()!;
      if (visibleKeys.has(item.key)) {
        surviving.push(item);
      }
    }
    for (const item of surviving) {
      this.queue.push(this.directionScore(item.tileRow, item.tileCol), item);
    }
  }

  async getTile(params: QueryTileParams, viewHash = ""): Promise<TileMeta> {
    const key = tileKey(params.row, params.col, viewHash);

    const cached = this.cache.get(key);
    if (cached) return { table: cached, isProvisional: false, jobId: null };

    const ts = this.failedAt.get(key);
    if (ts !== undefined) {
      if (Date.now() - ts < FAILED_TTL_MS) {
        return Promise.reject(new Error("tile unavailable"));
      }
      this.failedAt.delete(key);
    }

    const existing = this.inflight.get(key);
    if (existing) return existing.promise;

    const tileRow = Math.floor(params.row / TILE_ROWS);
    const tileCol = Math.floor(params.col / TILE_COLS);

    let capturedResolve!: (meta: TileMeta) => void;
    let capturedReject!: (e: unknown) => void;
    const promise = new Promise<TileMeta>((resolve, reject) => {
      capturedResolve = resolve;
      capturedReject = reject;
    });

    const entry: InflightEntry = { promise, reject: capturedReject, controller: null, enqueuedAt: Date.now() };
    this.inflight.set(key, entry);
    promise
      .then(() => this.inflight.delete(key))
      .catch(() => this.inflight.delete(key));

    this.enqueue({ key, tileRow, tileCol, params, resolve: capturedResolve, reject: capturedReject });
    this.scheduleDrain();
    return promise;
  }

  invalidate(): void {
    const err = new DOMException("AbortError", "AbortError");
    const remaining: QueueItem[] = [];
    while (this.queue.size > 0) {
      remaining.push(this.queue.pop()!);
    }
    for (const item of remaining) {
      item.reject(err);
    }
    for (const entry of this.inflight.values()) {
      if (entry.controller !== null) {
        entry.controller.abort();
      }
      entry.reject(err);
    }
    this.generation++;
    this.cache.clear();
    this.inflight.clear();
    this.failedAt.clear();
    this.retries.clear();
  }

  private enqueue(item: QueueItem): void {
    this.queue.push(this.directionScore(item.tileRow, item.tileCol), item);
  }

  private directionScore(tileRow: number, tileCol: number): number {
    const dr = tileRow - this.viewportCenterRow;
    const dc = tileCol - this.viewportCenterCol;
    const dist = Math.abs(dr) + Math.abs(dc);
    const dot = dr * this.vy + dc * this.vx;
    const landmarkBonus = this.landmarkTileRows.has(tileRow) ? -3 : 0;
    return dist - 0.5 * dot + landmarkBonus;
  }

  private startGC(): void {
    setInterval(() => {
      const now = Date.now();
      for (const [key, entry] of this.inflight) {
        if (now - entry.enqueuedAt > 45_000) {
          if (entry.controller !== null) entry.controller.abort();
          entry.reject(new DOMException("AbortError", "AbortError"));
          this.inflight.delete(key);
        }
      }
    }, 60_000);
  }

  private scheduleDrain(): void {
    if (this.drainScheduled) return;
    this.drainScheduled = true;
    requestAnimationFrame(() => {
      this.drainScheduled = false;
      this.drain();
    });
  }

  private scheduleRetry(item: QueueItem): void {
    const attempts = (this.retries.get(item.key) ?? 0) + 1;
    if (attempts >= MAX_RETRIES) {
      this.failedAt.set(item.key, Date.now());
      this.retries.delete(item.key);
      return;
    }
    this.retries.set(item.key, attempts);
    const capturedGen = this.generation;
    const delay = 500 * Math.pow(2, attempts - 1) + Math.random() * 500;
    setTimeout(() => {
      if (this.generation !== capturedGen) return;
      this.enqueue(item);
      this.drain();
    }, delay);
  }

  private drain(): void {
    while (this.active < MAX_CONCURRENCY && this.queue.size > 0) {
      const batch = this.collectBatch();
      if (batch.length === 0) break;

      if (batch.length === 1) {
        this.executeSingleTile(batch[0]!);
      } else {
        this.executeBatchTiles(batch);
      }
    }
  }

  private collectBatch(): QueueItem[] {
    if (this.queue.size === 0) return [];

    let first: QueueItem | undefined;
    while (this.queue.size > 0) {
      const candidate = this.queue.pop()!;
      if (this.inflight.has(candidate.key)) {
        first = candidate;
        break;
      }
    }
    if (!first) return [];

    const sourceId = first.params.viewExpr.source_id;
    const batch: QueueItem[] = [first];
    const deferred: QueueItem[] = [];

    while (this.queue.size > 0 && batch.length < MAX_CONCURRENCY) {
      const next = this.queue.pop()!;
      if (!this.inflight.has(next.key)) continue;
      if (next.params.viewExpr.source_id === sourceId) {
        batch.push(next);
      } else {
        deferred.push(next);
      }
    }

    for (const item of deferred) {
      this.queue.push(this.directionScore(item.tileRow, item.tileCol), item);
    }

    return batch;
  }

  private executeSingleTile(item: QueueItem): void {
    const entry = this.inflight.get(item.key);
    if (!entry) return;

    const gen = this.generation;
    const controller = new AbortController();
    entry.controller = controller;
    this.active++;

    fetchViewTile(item.params, controller.signal)
      .then((meta) => {
        if (this.generation !== gen) {
          item.reject(new DOMException("AbortError", "AbortError"));
          return;
        }
        this.retries.delete(item.key);
        this.cache.set(item.key, meta.table);
        item.resolve(meta);
      })
      .catch((err) => {
        if (this.generation !== gen) {
          item.reject(new DOMException("AbortError", "AbortError"));
          return;
        }
        if (err instanceof DOMException && err.name === "AbortError") {
          item.reject(err);
          return;
        }
        this.scheduleRetry(item);
        item.reject(err);
      })
      .finally(() => {
        this.active--;
        this.drain();
      });
  }

  private executeBatchTiles(batch: QueueItem[]): void {
    const gen = this.generation;
    const controller = new AbortController();

    for (const item of batch) {
      const entry = this.inflight.get(item.key);
      if (entry) entry.controller = controller;
    }

    this.active++;

    const viewExpr = batch[0]!.params.viewExpr;
    const tileRequests: BatchTileRequestItem[] = batch.map((item) => ({
      row: item.params.row,
      col: item.params.col,
      rows: item.params.rows ?? TILE_ROWS,
      cols: item.params.cols ?? TILE_COLS,
    }));

    const resolved = new Set<number>();
    fetchViewTileBatch(
      viewExpr,
      tileRequests,
      (idx, meta) => {
        if (this.generation !== gen) return;
        const item = batch[idx];
        if (!item) return;
        resolved.add(idx);
        this.retries.delete(item.key);
        this.cache.set(item.key, meta.table);
        item.resolve(meta);
      },
      controller.signal
    )
      .then(() => {
        if (this.generation !== gen) return;
        const serverErr = new Error("tile server error");
        for (let i = 0; i < batch.length; i++) {
          if (!resolved.has(i)) {
            const item = batch[i]!;
            this.scheduleRetry(item);
            item.reject(serverErr);
          }
        }
      })
      .catch((err) => {
        if (this.generation !== gen) {
          const abortErr = new DOMException("AbortError", "AbortError");
          for (const item of batch) item.reject(abortErr);
          return;
        }
        if (err instanceof DOMException && err.name === "AbortError") {
          for (const item of batch) item.reject(err);
          return;
        }
        for (const item of batch) {
          this.scheduleRetry(item);
          item.reject(err);
        }
      })
      .finally(() => {
        this.active--;
        this.drain();
      });
  }
}
