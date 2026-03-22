import { useUiStore } from "../../stores/ui";
import { useLandmarkStore } from "../../stores/landmarkStore";
import { useNavigation } from "../../hooks/useNavigation";
import { DEFAULT_CELL_H } from "../../lib/viewport";

const TYPE_COLORS: Record<string, string> = {
  null_surge: "#f59e0b",
  outlier: "#ef4444",
  boundary: "#3b82f6",
};

const TYPE_LABELS: Record<string, string> = {
  null_surge: "Null surge",
  outlier: "Outlier",
  boundary: "Boundary",
};

function NullSurgeIcon() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
      <circle cx="6" cy="6" r="5" stroke="#f59e0b" strokeWidth="1.5" />
      <line x1="6" y1="3" x2="6" y2="7" stroke="#f59e0b" strokeWidth="1.5" strokeLinecap="round" />
      <circle cx="6" cy="9" r="0.75" fill="#f59e0b" />
    </svg>
  );
}

function OutlierIcon() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
      <path d="M6 1.5L11 10.5H1L6 1.5Z" stroke="#ef4444" strokeWidth="1.5" strokeLinejoin="round" />
      <line x1="6" y1="5" x2="6" y2="7.5" stroke="#ef4444" strokeWidth="1.5" strokeLinecap="round" />
      <circle cx="6" cy="9" r="0.75" fill="#ef4444" />
    </svg>
  );
}

function BoundaryIcon() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
      <line x1="1" y1="6" x2="11" y2="6" stroke="#3b82f6" strokeWidth="1.5" strokeLinecap="round" strokeDasharray="2 2" />
    </svg>
  );
}

const ICONS: Record<string, React.ReactNode> = {
  null_surge: <NullSurgeIcon />,
  outlier: <OutlierIcon />,
  boundary: <BoundaryIcon />,
};

export function LandmarkPanel() {
  const showLandmarkPanel = useUiStore((s) => s.showLandmarkPanel);
  const setShowLandmarkPanel = useUiStore((s) => s.setShowLandmarkPanel);
  const zoom = useUiStore((s) => s.zoom);
  const landmarks = useLandmarkStore((s) => s.landmarks);
  const { teleportTo } = useNavigation();

  if (!showLandmarkPanel) return null;

  const sorted = [...landmarks].sort((a, b) => a.rowOffset - b.rowOffset);

  return (
    <div
      style={{
        position: "fixed",
        top: 80,
        right: 124,
        width: 280,
        maxHeight: 400,
        background: "var(--c-bg)",
        border: "1px solid var(--c-border)",
        borderRadius: 6,
        boxShadow: "0 8px 24px rgba(0,0,0,0.14)",
        zIndex: 500,
        display: "flex",
        flexDirection: "column",
        overflow: "hidden",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          padding: "8px 12px",
          borderBottom: "1px solid var(--c-border)",
          flexShrink: 0,
        }}
      >
        <span style={{ fontSize: 13, fontWeight: 600, color: "var(--c-text)" }}>Landmarks</span>
        <button
          onClick={() => setShowLandmarkPanel(false)}
          style={{ background: "none", border: "none", cursor: "pointer", color: "var(--c-text-3)", fontSize: 16, lineHeight: 1, padding: 0 }}
        >
          ×
        </button>
      </div>
      <div style={{ overflowY: "auto", flex: 1 }}>
        {sorted.length === 0 && (
          <div style={{ padding: 16, fontSize: 12, color: "var(--c-text-3)", textAlign: "center" }}>
            No landmarks detected
          </div>
        )}
        {sorted.map((lm, i) => (
          <button
            key={i}
            onClick={() => {
              teleportTo(0, lm.rowOffset * DEFAULT_CELL_H * zoom);
              setShowLandmarkPanel(false);
            }}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              width: "100%",
              padding: "7px 12px",
              background: "none",
              border: "none",
              borderBottom: "1px solid var(--c-border)",
              cursor: "pointer",
              textAlign: "left",
            }}
          >
            <span style={{ flexShrink: 0 }}>{ICONS[lm.type]}</span>
            <span style={{ flex: 1, fontSize: 12, color: "var(--c-text)" }}>
              Row {lm.rowOffset.toLocaleString()} — {TYPE_LABELS[lm.type] ?? lm.type}
            </span>
            <div
              style={{
                width: Math.round(lm.severity * 60),
                height: 4,
                borderRadius: 2,
                background: TYPE_COLORS[lm.type] ?? "#9ca3af",
                flexShrink: 0,
              }}
            />
          </button>
        ))}
      </div>
    </div>
  );
}
