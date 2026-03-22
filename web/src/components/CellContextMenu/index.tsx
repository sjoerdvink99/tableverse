import { useEffect, useRef } from "react";
import { useUiStore } from "../../stores/ui";
import { useViewStore } from "../../stores/view";
import { appendNotIn } from "../../lib/addPredicate";
import type { Literal } from "../../lib/viewExpr";

const MENU_STYLE: React.CSSProperties = {
  position: "fixed",
  background: "var(--c-bg)",
  border: "1px solid var(--c-border)",
  borderRadius: 8,
  boxShadow: "0 4px 16px rgba(0,0,0,0.2)",
  zIndex: 1000,
  minWidth: 200,
  padding: "4px 0",
  fontSize: 13,
};

const HEADER_STYLE: React.CSSProperties = {
  padding: "8px 12px 6px",
  color: "var(--c-text-3)",
  fontSize: 11,
  fontWeight: 600,
  letterSpacing: "0.04em",
  textTransform: "uppercase",
};

const DIVIDER: React.CSSProperties = {
  height: 1,
  background: "var(--c-surface-2)",
  margin: "3px 0",
};

function MenuItem({
  label,
  onClick,
}: {
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      className="tv-menu-item"
      onClick={onClick}
    >
      {label}
    </button>
  );
}

function toLiteral(value: unknown): Literal {
  if (value === null || value === undefined) return null;
  if (typeof value === "boolean") return value;
  if (typeof value === "number") return value;
  return String(value);
}

function formatValue(value: unknown): string {
  if (value === null || value === undefined) return "null";
  const s = String(value);
  return s.length > 24 ? s.slice(0, 24) + "…" : s;
}

export function CellContextMenu() {
  const contextMenu = useUiStore((s) => s.contextMenu);
  const setContextMenu = useUiStore((s) => s.setContextMenu);
  const addPredicate = useViewStore((s) => s.addPredicate);
  const setOps = useViewStore((s) => s.setOps);
  const ops = useViewStore((s) => s.ops);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setContextMenu(null);
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setContextMenu(null);
    };
    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [setContextMenu]);

  if (!contextMenu || contextMenu.kind !== "cell") return null;

  const { x, y, column, value } = contextMenu;
  const literal = toLiteral(value);
  const isNull = value === null || value === undefined;
  const isNumeric = typeof value === "number";
  const label = formatValue(value);

  const close = () => setContextMenu(null);

  const act = (fn: () => void) => {
    fn();
    close();
  };

  const menuX = Math.min(x, window.innerWidth - 220);
  const menuY = Math.min(y, window.innerHeight - 280);

  return (
    <div ref={menuRef} style={{ ...MENU_STYLE, left: menuX, top: menuY }}>
      <div style={HEADER_STYLE}>{column}</div>
      <div style={{ ...HEADER_STYLE, paddingTop: 0, color: "var(--c-text-2)", fontWeight: 400, textTransform: "none", letterSpacing: 0, fontSize: 12 }}>
        {isNull ? <em>null</em> : `"${label}"`}
      </div>
      <div style={DIVIDER} />

      {isNull ? (
        <>
          <MenuItem
            label={`Show rows where ${column} is null`}
            onClick={() => act(() => addPredicate({ op: "is_null", column }))}
          />
          <MenuItem
            label={`Hide rows where ${column} is null`}
            onClick={() => act(() => addPredicate({ op: "is_not_null", column }))}
          />
        </>
      ) : (
        <>
          <MenuItem
            label={`Filter: = ${label}`}
            onClick={() => act(() => addPredicate({ op: "eq", column, value: literal }))}
          />
          <MenuItem
            label={`Filter: ≠ ${label}`}
            onClick={() => act(() => addPredicate({ op: "ne", column, value: literal }))}
          />
          {isNumeric && (
            <>
              <MenuItem
                label={`Filter: > ${label}`}
                onClick={() => act(() => addPredicate({ op: "gt", column, value: literal }))}
              />
              <MenuItem
                label={`Filter: < ${label}`}
                onClick={() => act(() => addPredicate({ op: "lt", column, value: literal }))}
              />
            </>
          )}
          <div style={DIVIDER} />
          <MenuItem
            label={`Exclude "${label}"`}
            onClick={() => act(() => setOps(appendNotIn(ops, column, literal)))}
          />
        </>
      )}

      <div style={DIVIDER} />
      <MenuItem
        label="Copy value"
        onClick={() => act(() => navigator.clipboard.writeText(String(value ?? "")))}
      />
    </div>
  );
}
