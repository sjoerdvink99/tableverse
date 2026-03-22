import { useTableStore } from "../../stores/table";
import { formatRowCount } from "../../lib/format";
import { ROW_HEADER_W, HEADER_HEIGHT, DEFAULT_CELL_W, DEFAULT_CELL_H } from "../../lib/viewport";

export function CornerCell() {
  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);

  const rows = source?.n_rows ?? 0;
  const cols = source?.n_cols ?? 0;

  const firstRow = source ? Math.floor(viewport.scrollY / DEFAULT_CELL_H) + 1 : 0;
  const firstCol = source ? Math.floor(viewport.scrollX / DEFAULT_CELL_W) + 1 : 0;

  const maxScrollX = Math.max(1, cols * DEFAULT_CELL_W - viewport.width);
  const maxScrollY = Math.max(1, rows * DEFAULT_CELL_H - viewport.height);
  const pctX = source ? Math.min(100, Math.max(0, (viewport.scrollX / maxScrollX) * 100)) : 0;
  const pctY = source ? Math.min(100, Math.max(0, (viewport.scrollY / maxScrollY) * 100)) : 0;

  return (
    <div
      style={{
        width: ROW_HEADER_W,
        height: HEADER_HEIGHT,
        background: "var(--canvas-header-bg)",
        borderRight: "1px solid var(--canvas-grid)",
        borderBottom: "1px solid var(--canvas-grid)",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: 2,
        flexShrink: 0,
      }}
    >
      <span style={{ fontSize: 10, color: "var(--c-text-2)", lineHeight: 1.2 }}>
        {source ? `${formatRowCount(firstRow)}/${formatRowCount(rows)}` : "—"}
      </span>
      <span style={{ fontSize: 10, color: "var(--c-text-2)", lineHeight: 1.2 }}>
        {source ? `${firstCol}/${cols}` : "—"}
      </span>
      <div style={{ display: "flex", gap: 2, marginTop: 2 }}>
        <div style={{ width: 20, height: 3, background: "var(--c-border)", borderRadius: 2 }}>
          <div style={{ width: `${pctX}%`, height: "100%", background: "var(--c-accent)", borderRadius: 2 }} />
        </div>
        <div style={{ width: 3, height: 20, background: "var(--c-border)", borderRadius: 2 }}>
          <div style={{ height: `${pctY}%`, width: "100%", background: "var(--c-accent)", borderRadius: 2 }} />
        </div>
      </div>
    </div>
  );
}
