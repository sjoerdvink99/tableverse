import { useUiStore } from "../../stores/ui";
import { useZoom } from "../../hooks/useZoom";
import type { SatelliteEncoding } from "../../lib/profile-render";

function resolveModeName(zoom: number): string {
  if (zoom < 0.10) return "satellite";
  if (zoom < 0.28) return "profile";
  if (zoom < 0.55) return "heatmap";
  if (zoom < 0.85) return "scan";
  return "read";
}

const ENCODING_OPTIONS: { label: string; value: SatelliteEncoding }[] = [
  { label: "Nulls", value: "null_rate" },
  { label: "Values", value: "mean_normalized" },
  { label: "Variance", value: "spread" },
];

export function ZoomControls() {
  const zoom = useUiStore((s) => s.zoom);
  const satelliteEncoding = useUiStore((s) => s.satelliteEncoding);
  const setSatelliteEncoding = useUiStore((s) => s.setSatelliteEncoding);
  const { zoomIn, zoomOut, resetZoom } = useZoom();

  const modeName = resolveModeName(zoom);
  const isSatellite = zoom < 0.10;

  return (
    <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
      {isSatellite && (
        <div className="tv-encoding-group">
          {ENCODING_OPTIONS.map((opt) => (
            <button
              key={opt.value}
              className={`tv-encoding-btn${satelliteEncoding === opt.value ? " active" : ""}`}
              onClick={() => setSatelliteEncoding(opt.value)}
              title={`Satellite encoding: ${opt.label}`}
            >
              {opt.label}
            </button>
          ))}
        </div>
      )}
      <div className="tv-zoom-group">
        <button className="tv-zoom-btn" onClick={zoomOut} title="Zoom out (Ctrl+−)">
          −
        </button>
        <button className="tv-zoom-label" onClick={resetZoom} title="Reset zoom">
          {Math.round(zoom * 100)}%
        </button>
        <button className="tv-zoom-btn" onClick={zoomIn} title="Zoom in (Ctrl++)">
          +
        </button>
      </div>
      <span className="tv-zoom-mode">{modeName}</span>
    </div>
  );
}
