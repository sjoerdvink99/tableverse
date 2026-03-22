import { useEffect, useRef, useState } from "react";
import { useUiStore } from "../../stores/ui";
import { useViewStore } from "../../stores/view";

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

const DIVIDER: React.CSSProperties = {
  height: 1,
  background: "var(--c-surface-2)",
  margin: "3px 0",
};

function MenuItem({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      className="tv-menu-item"
      onClick={onClick}
    >
      {label}
    </button>
  );
}

export function ColumnContextMenu() {
  const contextMenu = useUiStore((s) => s.contextMenu);
  const setContextMenu = useUiStore((s) => s.setContextMenu);
  const addOp = useViewStore((s) => s.addOp);
  const setSort = useViewStore((s) => s.setSort);
  const ops = useViewStore((s) => s.ops);
  const menuRef = useRef<HTMLDivElement>(null);
  const [showFormula, setShowFormula] = useState(false);
  const [formula, setFormula] = useState("");
  const [deriveName, setDeriveName] = useState("");

  useEffect(() => {
    if (!contextMenu || contextMenu.kind !== "column") {
      setShowFormula(false);
      setFormula("");
      setDeriveName("");
    }
  }, [contextMenu]);

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

  if (!contextMenu || contextMenu.kind !== "column") return null;

  const { x, y, colName } = contextMenu;
  const close = () => setContextMenu(null);
  const act = (fn: () => void) => { fn(); close(); };

  const currentSort = ops.find((op) => op.type === "sort");
  const currentSortKeys = currentSort?.type === "sort" ? currentSort.keys : [];
  const colSort = currentSortKeys.find((k) => k.column === colName);

  const menuX = Math.min(x, window.innerWidth - 220);
  const menuY = Math.min(y, window.innerHeight - 320);

  const submitDerive = () => {
    if (!deriveName.trim() || !formula.trim()) return;
    addOp({
      type: "derive",
      name: deriveName.trim(),
      expr: { kind: "column", name: formula.trim() },
    });
    close();
  };

  return (
    <div ref={menuRef} style={{ ...MENU_STYLE, left: menuX, top: menuY }}>
      <MenuItem
        label={colSort?.descending === false ? "✓ Sort ascending" : "Sort ascending"}
        onClick={() => act(() => setSort([{ column: colName, descending: false, nulls_last: true }]))}
      />
      <MenuItem
        label={colSort?.descending === true ? "✓ Sort descending" : "Sort descending"}
        onClick={() => act(() => setSort([{ column: colName, descending: true, nulls_last: true }]))}
      />
      <div style={DIVIDER} />
      <MenuItem
        label="Hide column"
        onClick={() => act(() => addOp({ type: "drop", columns: [colName] }))}
      />
      <div style={DIVIDER} />
      <MenuItem
        label="Group by this column"
        onClick={() => act(() => addOp({ type: "group_by", keys: [colName], aggs: [{ fn: "count", alias: "n" }] }))}
      />
      <MenuItem
        label="Value counts"
        onClick={() => act(() => addOp({ type: "group_by", keys: [colName], aggs: [{ fn: "count", alias: "n" }] }))}
      />
      <div style={DIVIDER} />
      <MenuItem
        label="Add calculation..."
        onClick={() => setShowFormula(true)}
      />
      {showFormula && (
        <div style={{ padding: "8px 10px", display: "flex", flexDirection: "column", gap: 6 }}>
          <input
            autoFocus
            placeholder="Expression (e.g. price * qty)"
            value={formula}
            onChange={(e) => setFormula(e.target.value)}
            style={{
              width: "100%",
              padding: "5px 8px",
              border: "1px solid var(--c-border)",
              borderRadius: 5,
              fontSize: 12,
              outline: "none",
              boxSizing: "border-box",
              background: "var(--c-bg)",
              color: "var(--c-text)",
            }}
          />
          <input
            placeholder="New column name"
            value={deriveName}
            onChange={(e) => setDeriveName(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") submitDerive(); }}
            style={{
              width: "100%",
              padding: "5px 8px",
              border: "1px solid var(--c-border)",
              borderRadius: 5,
              fontSize: 12,
              outline: "none",
              boxSizing: "border-box",
              background: "var(--c-bg)",
              color: "var(--c-text)",
            }}
          />
          <button
            onClick={submitDerive}
            style={{
              padding: "5px 10px",
              background: "var(--c-accent)",
              color: "#fff",
              border: "none",
              borderRadius: 5,
              fontSize: 12,
              cursor: "pointer",
            }}
          >
            Add
          </button>
        </div>
      )}
    </div>
  );
}
