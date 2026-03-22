export type UrlState = {
  sourceId: string;
  ops: string;
  sx: number;
  sy: number;
  zoom: number;
};

export function encodeUrlState(state: Partial<UrlState>): string {
  const params = new URLSearchParams();
  if (state.sourceId) params.set("source", state.sourceId);
  if (state.ops) params.set("ops", btoa(state.ops));
  if (state.sx !== undefined) params.set("sx", String(Math.round(state.sx)));
  if (state.sy !== undefined) params.set("sy", String(Math.round(state.sy)));
  if (state.zoom !== undefined) params.set("z", String(state.zoom));
  return params.toString();
}

export function decodeUrlState(): Partial<UrlState> {
  const hash = window.location.hash.slice(1);
  if (!hash) return {};
  const params = new URLSearchParams(hash);
  const result: Partial<UrlState> = {};
  const source = params.get("source");
  if (source) result.sourceId = source;
  const ops = params.get("ops");
  if (ops) {
    try { result.ops = atob(ops); } catch {}
  }
  const sx = params.get("sx");
  if (sx) result.sx = Number(sx);
  const sy = params.get("sy");
  if (sy) result.sy = Number(sy);
  const z = params.get("z");
  if (z) result.zoom = Number(z);
  return result;
}
