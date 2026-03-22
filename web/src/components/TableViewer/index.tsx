import { useRef } from "react";
import { useTableStore } from "../../stores/table";
import { useViewport } from "../../hooks/useViewport";
import { useTiles } from "../../hooks/useTiles";
import { useKeyboard } from "../../hooks/useKeyboard";
import { useRowCount } from "../../hooks/useRowCount";
import { useVirtualSchema } from "../../hooks/useVirtualSchema";
import { useSemanticStats } from "../../hooks/useSemanticStats";
import { useRowGroupStats } from "../../hooks/useRowGroupStats";
import { useCorrelations } from "../../hooks/useCorrelations";
import { ColumnHeader } from "./ColumnHeader";
import { RowHeader } from "./RowHeader";
import { CornerCell } from "./CornerCell";
import { ScrollContainer } from "./ScrollContainer";
import { ColumnDistributionPopover } from "../ColumnDistributionPopover";
import { Minimap } from "../Minimap";
import { ROW_HEADER_W, HEADER_HEIGHT } from "../../lib/viewport";

export function TableViewer() {
  const containerRef = useRef<HTMLDivElement>(null);
  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);

  useViewport(containerRef);
  useTiles();
  useKeyboard();
  useRowCount();
  useVirtualSchema();
  useSemanticStats();
  useRowGroupStats();
  useCorrelations();

  const gridW = Math.max(0, viewport.width - ROW_HEADER_W);
  const gridH = Math.max(0, viewport.height - HEADER_HEIGHT);

  if (!source) {
    return (
      <div
        ref={containerRef}
        style={{
          flex: 1,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          color: "var(--c-text-3)",
          fontSize: 14,
          background: "var(--c-bg)",
        }}
      >
        No source loaded. Add a data source to get started.
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      style={{
        flex: 1,
        display: "grid",
        gridTemplateColumns: `${ROW_HEADER_W}px 1fr`,
        gridTemplateRows: `${HEADER_HEIGHT}px 1fr`,
        overflow: "hidden",
        background: "var(--canvas-bg)",
        border: "1px solid var(--c-border)",
        position: "relative",
      }}
    >
      <CornerCell />
      <ColumnHeader width={gridW} leftOffset={ROW_HEADER_W} />
      <RowHeader height={gridH} />
      <ScrollContainer width={gridW} height={gridH} />
      <ColumnDistributionPopover />
      <Minimap />
    </div>
  );
}
