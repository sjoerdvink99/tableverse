import { useState } from "react";
import { useUiStore } from "../../stores/ui";
import { useTableStore } from "../../stores/table";
import { DEFAULT_CELL_H } from "../../lib/viewport";

export function JumpToRow() {
  const show = useUiStore((s) => s.showJumpToRow);
  const setShow = useUiStore((s) => s.setShowJumpToRow);
  const source = useTableStore((s) => s.source);
  const setViewport = useTableStore((s) => s.setViewport);
  const [value, setValue] = useState("");

  if (!show) return null;

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const row = parseInt(value, 10);
    if (!isNaN(row) && row >= 1) {
      setViewport({ scrollY: (row - 1) * DEFAULT_CELL_H });
    }
    setShow(false);
    setValue("");
  };

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.4)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 900,
      }}
      onClick={() => setShow(false)}
    >
      <div
        style={{
          background: "var(--c-bg)",
          borderRadius: 8,
          padding: "20px 24px",
          width: 300,
          boxShadow: "0 8px 32px rgba(0,0,0,0.3)",
          border: "1px solid var(--c-border)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <h3 style={{ margin: "0 0 12px", fontSize: 15, color: "var(--c-text)" }}>Jump to row</h3>
        <form onSubmit={handleSubmit} style={{ display: "flex", gap: 8 }}>
          <input
            autoFocus
            type="number"
            min={1}
            max={source?.n_rows ?? undefined}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder={`1 – ${source?.n_rows ?? "?"}`}
            style={{ flex: 1, padding: "6px 10px", fontSize: 14, border: "1px solid var(--c-border)", borderRadius: 4, background: "var(--c-bg)", color: "var(--c-text)", outline: "none" }}
          />
          <button type="submit" style={{ padding: "6px 14px", background: "var(--c-accent)", color: "#fff", border: "none", borderRadius: 4, cursor: "pointer", fontSize: 13 }}>
            Go
          </button>
        </form>
      </div>
    </div>
  );
}
