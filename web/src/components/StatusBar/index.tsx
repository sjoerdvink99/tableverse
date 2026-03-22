import { useTableStore } from "../../stores/table";
import { useViewStore } from "../../stores/view";
import { useUiStore } from "../../stores/ui";
import { usePerfStore } from "../../stores/perf";
import { formatRowCount } from "../../lib/format";
import { DEFAULT_CELL_W, DEFAULT_CELL_H } from "../../lib/viewport";

export function StatusBar() {
  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);
  const virtualRowCount = useViewStore((s) => s.virtualRowCount);
  const virtualSchema = useViewStore((s) => s.virtualSchema);
  const ops = useViewStore((s) => s.ops);
  const zoom = useUiStore((s) => s.zoom);
  const lastTileMs = usePerfStore((s) => s.lastTileMs);
  const hitRatio = usePerfStore((s) => s.hitRatio);

  if (!source) {
    return (
      <div style={{ height: 24, background: "var(--c-surface)", borderTop: "1px solid var(--c-border)", flexShrink: 0 }} />
    );
  }

  const filterCount = ops.filter((op) => op.type === "filter").length;
  const colCount = virtualSchema !== null ? virtualSchema.length : source.n_cols;
  const rowLabel =
    virtualRowCount !== null && virtualRowCount !== source.n_rows
      ? `${formatRowCount(virtualRowCount)} of ${formatRowCount(source.n_rows)} rows`
      : `${formatRowCount(source.n_rows)} rows`;

  const visibleRows = Math.max(1, Math.ceil(viewport.height / (DEFAULT_CELL_H * zoom)));
  const visibleCols = Math.max(1, Math.ceil(viewport.width / (DEFAULT_CELL_W * zoom)));

  const parts: string[] = [
    rowLabel,
    `${colCount} column${colCount !== 1 ? "s" : ""}`,
  ];
  if (filterCount > 0) {
    parts.push(`${filterCount} filter${filterCount !== 1 ? "s" : ""}`);
  }
  if (zoom < 1.0) {
    parts.push(`viewing ${visibleRows.toLocaleString()} × ${visibleCols} cells`);
  }

  return (
    <div
      style={{
        height: 24,
        background: "var(--c-surface)",
        borderTop: "1px solid var(--c-border)",
        display: "flex",
        alignItems: "center",
        padding: "0 12px",
        gap: 16,
        fontSize: 11.5,
        color: "var(--c-text-2)",
        flexShrink: 0,
      }}
    >
      <span>{parts.join(" · ")}</span>
      {lastTileMs > 0 && (
        <span style={{ marginLeft: "auto" }}>
          tile: {lastTileMs}ms · cache: {Math.round(hitRatio() * 100)}%
        </span>
      )}
    </div>
  );
}
