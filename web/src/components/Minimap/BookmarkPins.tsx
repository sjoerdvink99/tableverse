import { useTableStore } from "../../stores/table";
import { useUiStore } from "../../stores/ui";
import { useViewStore } from "../../stores/view";
import { useLandmarkStore } from "../../stores/landmarkStore";
import { useBookmarkStore } from "../../stores/bookmarkStore";
import { DEFAULT_CELL_W, DEFAULT_CELL_H, scrollBounds } from "../../lib/viewport";

interface Props {
  panelW: number;
  panelH: number;
}

const LANDMARK_COLORS: Record<string, string> = {
  null_surge: "#f59e0b",
  outlier: "#ef4444",
  boundary: "#3b82f6",
};

const LANDMARK_LABELS: Record<string, string> = {
  null_surge: "Null surge",
  outlier: "Statistical outlier",
  boundary: "Row group boundary",
};

export function BookmarkPins({ panelW, panelH }: Props) {
  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);
  const zoom = useUiStore((s) => s.zoom);
  const virtualRowCount = useViewStore((s) => s.virtualRowCount);
  const landmarks = useLandmarkStore((s) => s.landmarks);
  const bookmarks = useBookmarkStore((s) => s.bookmarks);

  if (!source) return null;

  const nRows = virtualRowCount ?? source.n_rows;
  const nCols = source.n_cols;
  const cellW = DEFAULT_CELL_W * zoom;
  const cellH = DEFAULT_CELL_H * zoom;
  const { maxY } = scrollBounds(nRows, nCols, cellW, cellH, viewport.width, viewport.height);

  return (
    <svg
      style={{ position: "absolute", top: 0, left: 0, width: panelW, height: panelH, pointerEvents: "none" }}
      width={panelW}
      height={panelH}
    >
      {landmarks.map((lm, i) => {
        const ly = (lm.rowOffset / nRows) * panelH;
        const color = LANDMARK_COLORS[lm.type] ?? "#9ca3af";
        const label = LANDMARK_LABELS[lm.type] ?? lm.type;
        return (
          <line
            key={`lm-${i}`}
            x1={panelW - 8}
            y1={ly}
            x2={panelW}
            y2={ly}
            stroke={color}
            strokeWidth={2}
            opacity={0.5}
          >
            <title>{`${label} — Row ${lm.rowOffset.toLocaleString()} (severity: ${(lm.severity * 100).toFixed(0)}%)`}</title>
          </line>
        );
      })}
      {bookmarks.map((bm) => {
        const bx = 8;
        const by = maxY > 0 ? (bm.scrollY / maxY) * (panelH - 8) : 0;
        const s = 5;
        const points = `${bx},${by - s} ${bx + s},${by} ${bx},${by + s} ${bx - s},${by}`;
        return (
          <polygon key={bm.id} points={points} fill={bm.color} opacity={0.9}>
            <title>{bm.label}</title>
          </polygon>
        );
      })}
    </svg>
  );
}
