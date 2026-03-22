import { useUiStore } from "../../stores/ui";

export function Tooltip() {
  const tooltip = useUiStore((s) => s.tooltip);
  if (!tooltip) return null;

  return (
    <div
      style={{
        position: "fixed",
        left: tooltip.x,
        top: tooltip.y,
        background: "var(--c-text)",
        color: "var(--c-bg)",
        padding: "4px 8px",
        borderRadius: 4,
        fontSize: 12,
        maxWidth: 320,
        wordBreak: "break-all",
        pointerEvents: "none",
        zIndex: 1000,
        boxShadow: "0 2px 8px rgba(0,0,0,0.3)",
      }}
    >
      {tooltip.value || <em style={{ color: "var(--c-text-3)", fontStyle: "italic" }}>null</em>}
    </div>
  );
}
