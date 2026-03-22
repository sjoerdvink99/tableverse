import type { ColumnStats, CorrelationMatrix, Credentials, RowGroupStat, SearchResults, SourceMeta } from "./types";
import type { ViewExpr } from "./viewExpr";
import { tableFromIPC } from "apache-arrow";
import type { Table } from "apache-arrow";

const BASE = (import.meta.env.VITE_API_BASE as string | undefined) ?? "";

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`);
  if (!res.ok) throw new Error(`GET ${path} failed: ${res.status}`);
  return res.json();
}

async function post<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`POST ${path} failed: ${res.status}`);
  return res.json();
}

async function del(path: string): Promise<void> {
  const res = await fetch(`${BASE}${path}`, { method: "DELETE" });
  if (!res.ok) throw new Error(`DELETE ${path} failed: ${res.status}`);
}

export async function fetchSources(): Promise<SourceMeta[]> {
  return get("/api/v1/sources");
}

export async function registerSource(
  uri: string,
  name?: string,
  profile?: string,
  credentials?: Credentials
): Promise<SourceMeta> {
  return post("/api/v1/sources", { uri, name, profile, credentials });
}

export async function fetchProfiles(): Promise<string[]> {
  return get("/api/v1/profiles");
}

export async function getSource(id: string): Promise<SourceMeta> {
  return get(`/api/v1/sources/${id}`);
}

export async function deleteSource(id: string): Promise<void> {
  return del(`/api/v1/sources/${id}`);
}

export type QueryTileParams = {
  viewExpr: ViewExpr;
  row: number;
  col: number;
  rows?: number;
  cols?: number;
  mode?: "agg";
};

export type TileMeta = {
  table: Table;
  isProvisional: boolean;
  jobId: string | null;
};

export async function fetchViewTile(params: QueryTileParams, signal?: AbortSignal): Promise<TileMeta> {
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), 30_000);
  const combined = signal
    ? typeof AbortSignal.any === "function"
      ? AbortSignal.any([signal, controller.signal])
      : controller.signal
    : controller.signal;
  try {
    const res = await fetch(`${BASE}/api/v1/sources/${params.viewExpr.source_id}/query/tiles`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        view_expr: params.viewExpr,
        row: params.row,
        col: params.col,
        rows: params.rows,
        cols: params.cols,
        mode: params.mode,
      }),
      signal: combined,
    });
    if (!res.ok) throw new Error(`fetchViewTile failed: ${res.status}`);
    const isProvisional = res.headers.get("x-tv-tile-status") === "provisional";
    const jobId = res.headers.get("x-tv-job-id");
    const table = tableFromIPC(await res.arrayBuffer());
    return { table, isProvisional, jobId };
  } finally {
    clearTimeout(timeoutId);
  }
}

export type BatchTileRequestItem = {
  row: number;
  col: number;
  rows: number;
  cols: number;
};

export async function fetchViewTileBatch(
  viewExpr: ViewExpr,
  tiles: BatchTileRequestItem[],
  onTile: (idx: number, meta: TileMeta) => void,
  signal?: AbortSignal
): Promise<void> {
  const res = await fetch(`${BASE}/api/v1/sources/${viewExpr.source_id}/query/tiles/batch`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ view_expr: viewExpr, tiles }),
    signal,
  });
  if (!res.ok || !res.body) throw new Error(`fetchViewTileBatch failed: ${res.status}`);
  const reader = res.body.getReader();
  let pending = new Uint8Array(0);
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    const merged = new Uint8Array(pending.length + value.length);
    merged.set(pending);
    merged.set(value, pending.length);
    pending = merged;
    while (pending.length >= 8) {
      const tileIdx =
        (pending[0]! | (pending[1]! << 8) | (pending[2]! << 16) | (pending[3]! << 24)) >>> 0;
      const ipcLen =
        (pending[4]! | (pending[5]! << 8) | (pending[6]! << 16) | (pending[7]! << 24)) >>> 0;
      if (pending.length < 8 + ipcLen) break;
      const MAX_IPC_FRAME_BYTES = 512 * 1024 * 1024;
      if (ipcLen > MAX_IPC_FRAME_BYTES) {
        pending = pending.subarray(8 + ipcLen);
        continue;
      }
      if (ipcLen > 0) {
        try {
          const ipcBytes = pending.buffer.slice(
            pending.byteOffset + 8,
            pending.byteOffset + 8 + ipcLen
          );
          onTile(tileIdx, { table: tableFromIPC(ipcBytes), isProvisional: false, jobId: null });
        } catch { }
      }
      pending = pending.subarray(8 + ipcLen);
    }
  }
}

export async function fetchViewCount(viewExpr: ViewExpr): Promise<number> {
  const data = await post<{ count: number }>(
    `/api/v1/sources/${viewExpr.source_id}/query/count`,
    { view_expr: viewExpr }
  );
  return data.count;
}

export async function fetchViewSchema(viewExpr: ViewExpr): Promise<SourceMeta["columns"]> {
  const data = await post<{ columns: SourceMeta["columns"] }>(
    `/api/v1/sources/${viewExpr.source_id}/query/schema`,
    { view_expr: viewExpr }
  );
  return data.columns;
}

export type ExportFormat = "sql" | "python_duckdb" | "python_polars" | "python_pandas" | "shell" | "shell_csv" | "ansi_sql" | "dbt";

export async function fetchExportCode(
  viewExpr: ViewExpr,
  format: ExportFormat
): Promise<string> {
  const data = await post<{ code: string }>(
    `/api/v1/sources/${viewExpr.source_id}/query/export`,
    { view_expr: viewExpr, format }
  );
  return data.code;
}

export function buildDownloadUrl(viewExpr: ViewExpr, format: "parquet" | "csv" | "arrow" | "jsonl"): string {
  const encoded = btoa(JSON.stringify(viewExpr));
  return `${BASE}/api/v1/sources/${viewExpr.source_id}/query/download?format=${format}&view_expr=${encoded}`;
}

export async function fetchColumnStats(sourceId: string, colIdx: number, bins?: number): Promise<ColumnStats> {
  const query = bins !== undefined ? `?bins=${bins}` : "";
  return get(`/api/v1/sources/${sourceId}/columns/${colIdx}/stats${query}`);
}

export async function fetchProfile(sourceId: string): Promise<ColumnStats[]> {
  return get(`/api/v1/sources/${sourceId}/profile`);
}

export async function fetchCorrelations(sourceId: string): Promise<CorrelationMatrix> {
  return get(`/api/v1/sources/${sourceId}/correlations`);
}

export async function fetchRowGroupStats(sourceId: string, colIdx: number): Promise<RowGroupStat[]> {
  return get(`/api/v1/sources/${sourceId}/columns/${colIdx}/row-group-stats`);
}

export async function fetchRowGroupStatsBatch(
  sourceId: string,
  colIndices: number[],
): Promise<Record<string, RowGroupStat[]>> {
  if (colIndices.length === 0) return {};
  const cols = colIndices.join(",");
  return get(`/api/v1/sources/${sourceId}/row-group-stats/batch?cols=${cols}`);
}

export async function searchSource(
  sourceId: string,
  query: string,
  columns?: string[],
  limit = 100
): Promise<SearchResults> {
  return post(`/api/v1/sources/${sourceId}/search`, { query, columns, limit });
}

export type StatsEvent =
  | { type: "metadata"; min: unknown; max: unknown; null_count: number; row_count: number; col_name: string | null }
  | { type: "histogram_coarse"; data: ColumnStats }
  | { type: "stats"; data: ColumnStats }
  | { type: "done" }
  | { type: "error"; message: string };

export function subscribeColumnStats(
  sourceId: string,
  colIdx: number,
  bins: number,
  onEvent: (event: StatsEvent) => void,
  onCoarse?: (stats: ColumnStats) => void,
): () => void {
  const url = `${BASE}/api/v1/sources/${sourceId}/columns/${colIdx}/stats/stream?bins=${bins}`;
  const es = new EventSource(url);

  es.addEventListener("metadata", (e) => {
    try {
      const data = JSON.parse((e as MessageEvent).data);
      onEvent({ type: "metadata", ...data });
    } catch {
    }
  });

  es.addEventListener("histogram_coarse", (e) => {
    try {
      const coarse = JSON.parse((e as MessageEvent).data) as ColumnStats;
      onEvent({ type: "histogram_coarse", data: coarse });
      onCoarse?.(coarse);
    } catch {
    }
  });

  es.addEventListener("stats", (e) => {
    try {
      const data = JSON.parse((e as MessageEvent).data) as ColumnStats;
      onEvent({ type: "stats", data });
    } catch {
    }
  });

  es.addEventListener("done", () => {
    onEvent({ type: "done" });
    es.close();
  });

  es.addEventListener("error", (e) => {
    onEvent({ type: "error", message: (e as MessageEvent).data ?? "unknown error" });
    es.close();
  });

  return () => es.close();
}

export async function speculativeSort(
  sourceId: string,
  viewExpr: import("./viewExpr").ViewExpr,
  colName: string
): Promise<void> {
  const sortOp = { type: "sort" as const, keys: [{ column: colName, descending: false, nulls_last: true }] };
  const anticipatedExpr = { ...viewExpr, ops: [...viewExpr.ops, sortOp] };
  try {
    await fetch(`${BASE}/api/v1/sources/${sourceId}/query/tiles`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        view_expr: anticipatedExpr,
        row: 0,
        col: 0,
        rows: 256,
        cols: 64,
      }),
      signal: AbortSignal.timeout(2000),
    });
  } catch {
  }
}
