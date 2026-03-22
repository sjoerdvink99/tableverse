import { useViewStore } from "../../stores/view";
import { opLabel } from "../../lib/viewExpr";
import type { ViewOp } from "../../lib/viewExpr";

const CHIP_BASE: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 5,
  height: 26,
  padding: "0 8px",
  borderRadius: 5,
  fontSize: 12,
  fontWeight: 500,
  cursor: "default",
  userSelect: "none",
  whiteSpace: "nowrap",
};

const REMOVE_BTN: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: 14,
  height: 14,
  borderRadius: 3,
  background: "transparent",
  border: "none",
  cursor: "pointer",
  padding: 0,
  color: "inherit",
  opacity: 0.6,
  fontSize: 11,
  lineHeight: 1,
};

function OpChip({
  label,
  color,
  onRemove,
}: {
  label: string;
  color: string;
  onRemove: () => void;
}) {
  return (
    <span
      style={{
        ...CHIP_BASE,
        background: color,
        color: "var(--c-text)",
        border: "1px solid var(--c-border)",
      }}
    >
      {label}
      <button
        style={REMOVE_BTN}
        onClick={onRemove}
        title="Remove"
      >
        ×
      </button>
    </span>
  );
}

function chipColor(op: ViewOp): string {
  switch (op.type) {
    case "filter": return "var(--chip-filter-bg)";
    case "sort": return "var(--chip-sort-bg)";
    case "derive": return "var(--chip-derive-bg)";
    case "group_by": return "var(--chip-group-bg)";
    case "sample":
    case "deduplicate": return "var(--chip-sample-bg)";
    default: return "var(--chip-default-bg)";
  }
}

export function PipelineBar() {
  const ops = useViewStore((s) => s.ops);
  const removeOp = useViewStore((s) => s.removeOp);
  const virtualRowCount = useViewStore((s) => s.virtualRowCount);

  if (ops.length === 0) return null;

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 6,
        padding: "6px 12px",
        background: "var(--c-surface)",
        borderBottom: "1px solid var(--c-border)",
        overflowX: "auto",
        flexShrink: 0,
        minHeight: 40,
      }}
    >
      {ops.map((op, i) => (
        <OpChip
          key={i}
          label={opLabel(op)}
          color={chipColor(op)}
          onRemove={() => removeOp(i)}
        />
      ))}

      {virtualRowCount !== null && (
        <span
          style={{
            marginLeft: "auto",
            fontSize: 11,
            color: "var(--c-text-2)",
            whiteSpace: "nowrap",
            flexShrink: 0,
          }}
        >
          {virtualRowCount.toLocaleString()} rows
        </span>
      )}
    </div>
  );
}
