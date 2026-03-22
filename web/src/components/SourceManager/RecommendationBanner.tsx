import type { SourceRecommendation } from "../../lib/types";

type Props = {
  recommendations: SourceRecommendation[];
  onDismiss: () => void;
};

export function RecommendationBanner({ recommendations, onDismiss }: Props) {
  if (recommendations.length === 0) return null;

  return (
    <div
      style={{
        position: "fixed",
        top: 16,
        left: "50%",
        transform: "translateX(-50%)",
        zIndex: 1100,
        background: "#fffbeb",
        border: "1px solid #f59e0b",
        borderRadius: 8,
        padding: "12px 16px",
        maxWidth: 520,
        width: "calc(100vw - 32px)",
        boxShadow: "0 4px 16px rgba(0,0,0,0.12)",
        display: "flex",
        flexDirection: "column",
        gap: 8,
      }}
    >
      <div style={{ display: "flex", alignItems: "flex-start", justifyContent: "space-between", gap: 8 }}>
        <div style={{ fontWeight: 600, fontSize: 13, color: "#92400e" }}>
          Source recommendations
        </div>
        <button
          onClick={onDismiss}
          style={{
            background: "none",
            border: "none",
            cursor: "pointer",
            fontSize: 16,
            color: "#b45309",
            lineHeight: 1,
            padding: "0 2px",
            flexShrink: 0,
          }}
          aria-label="Dismiss"
        >
          x
        </button>
      </div>
      <ul style={{ margin: 0, padding: "0 0 0 16px", display: "flex", flexDirection: "column", gap: 4 }}>
        {recommendations.map((rec, i) => (
          <li key={i} style={{ fontSize: 12, color: "#78350f" }}>
            <span
              style={{
                fontFamily: "ui-monospace, monospace",
                fontSize: 11,
                background: "#fde68a",
                color: "#92400e",
                borderRadius: 3,
                padding: "0 4px",
                marginRight: 6,
              }}
            >
              {rec.kind}
            </span>
            {rec.message}
          </li>
        ))}
      </ul>
    </div>
  );
}
