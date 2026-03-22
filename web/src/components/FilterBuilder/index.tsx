import { useState } from "react";
import { useUiStore } from "../../stores/ui";
import { useViewStore } from "../../stores/view";
import { useTableStore } from "../../stores/table";
import type { Predicate } from "../../lib/viewExpr";

type FilterRow = {
  id: string;
  column: string;
  operator: string;
  value: string;
};

type Combinator = "and" | "or";

const OPERATORS = [
  { value: "eq", label: "=" },
  { value: "ne", label: "≠" },
  { value: "gt", label: ">" },
  { value: "gte", label: ">=" },
  { value: "lt", label: "<" },
  { value: "lte", label: "<=" },
  { value: "contains", label: "contains" },
  { value: "starts_with", label: "starts with" },
  { value: "ends_with", label: "ends with" },
  { value: "is_null", label: "is null" },
  { value: "is_not_null", label: "is not null" },
];

const NO_VALUE_OPS = new Set(["is_null", "is_not_null"]);

function parseValue(raw: string): string | number | boolean | null {
  if (raw === "null") return null;
  if (raw === "true") return true;
  if (raw === "false") return false;
  const n = Number(raw);
  if (!isNaN(n) && raw.trim() !== "") return n;
  return raw;
}

function rowToPredicate(row: FilterRow): Predicate {
  switch (row.operator) {
    case "eq": return { op: "eq", column: row.column, value: parseValue(row.value) };
    case "ne": return { op: "ne", column: row.column, value: parseValue(row.value) };
    case "gt": return { op: "gt", column: row.column, value: parseValue(row.value) };
    case "gte": return { op: "gte", column: row.column, value: parseValue(row.value) };
    case "lt": return { op: "lt", column: row.column, value: parseValue(row.value) };
    case "lte": return { op: "lte", column: row.column, value: parseValue(row.value) };
    case "contains": return { op: "contains", column: row.column, value: row.value };
    case "starts_with": return { op: "starts_with", column: row.column, value: row.value };
    case "ends_with": return { op: "ends_with", column: row.column, value: row.value };
    case "is_null": return { op: "is_null", column: row.column };
    case "is_not_null": return { op: "is_not_null", column: row.column };
    default: return { op: "eq", column: row.column, value: parseValue(row.value) };
  }
}

function newRow(defaultColumn: string): FilterRow {
  return { id: String(Date.now() + Math.random()), column: defaultColumn, operator: "eq", value: "" };
}

const INPUT_STYLE: React.CSSProperties = {
  padding: "5px 8px",
  fontSize: 12,
  border: "1px solid var(--c-border)",
  borderRadius: 4,
  background: "var(--c-bg)",
  color: "var(--c-text)",
  outline: "none",
};

const SELECT_STYLE: React.CSSProperties = {
  ...INPUT_STYLE,
  cursor: "pointer",
};

export function FilterBuilder() {
  const showFilterBuilder = useUiStore((s) => s.showFilterBuilder);
  const setShowFilterBuilder = useUiStore((s) => s.setShowFilterBuilder);
  const source = useTableStore((s) => s.source);
  const setOps = useViewStore((s) => s.setOps);
  const ops = useViewStore((s) => s.ops);

  const columns = source?.columns ?? [];
  const defaultColumn = columns[0]?.name ?? "";

  const [rows, setRows] = useState<FilterRow[]>([newRow(defaultColumn)]);
  const [combinator, setCombinator] = useState<Combinator>("and");

  if (!showFilterBuilder) return null;

  const addRow = () => {
    setRows((r) => [...r, newRow(defaultColumn)]);
  };

  const removeRow = (id: string) => {
    setRows((r) => r.filter((row) => row.id !== id));
  };

  const updateRow = (id: string, patch: Partial<FilterRow>) => {
    setRows((r) => r.map((row) => (row.id === id ? { ...row, ...patch } : row)));
  };

  const handleApply = () => {
    const validRows = rows.filter((r) => r.column && (NO_VALUE_OPS.has(r.operator) || r.value.trim() !== ""));
    if (validRows.length === 0) return;

    const predicates = validRows.map(rowToPredicate);
    const first = predicates[0];
    if (!first) return;
    const combined: Predicate =
      predicates.length === 1
        ? first
        : { op: combinator, exprs: predicates };

    const withoutFilters = ops.filter((op) => op.type !== "filter");
    setOps([...withoutFilters, { type: "filter", predicate: combined }]);
    setShowFilterBuilder(false);
  };

  const handleClear = () => {
    const withoutFilters = ops.filter((op) => op.type !== "filter");
    setOps(withoutFilters);
    setRows([newRow(defaultColumn)]);
  };

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.4)",
        zIndex: 2500,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
      }}
      onClick={() => setShowFilterBuilder(false)}
    >
      <div
        style={{
          background: "var(--c-bg)",
          borderRadius: 10,
          width: "90%",
          maxWidth: 620,
          maxHeight: "70vh",
          display: "flex",
          flexDirection: "column",
          boxShadow: "0 16px 48px rgba(0,0,0,0.4)",
          overflow: "hidden",
          border: "1px solid var(--c-border)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            padding: "14px 20px 12px",
            borderBottom: "1px solid var(--c-border)",
            flexShrink: 0,
          }}
        >
          <span style={{ fontSize: 15, fontWeight: 600, color: "var(--c-text)" }}>Filter Builder</span>
          <button
            onClick={() => setShowFilterBuilder(false)}
            style={{ background: "none", border: "none", fontSize: 18, cursor: "pointer", color: "var(--c-text-3)", lineHeight: 1, padding: "2px 6px" }}
          >
            ×
          </button>
        </div>

        <div style={{ padding: "12px 20px", borderBottom: "1px solid var(--c-border)", flexShrink: 0, display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ fontSize: 12, color: "var(--c-text-2)" }}>Combine with</span>
          {(["and", "or"] as Combinator[]).map((c) => (
            <button
              key={c}
              onClick={() => setCombinator(c)}
              style={{
                padding: "3px 10px",
                fontSize: 12,
                fontWeight: 500,
                border: "1px solid",
                borderRadius: 4,
                cursor: "pointer",
                background: combinator === c ? "var(--c-accent)" : "var(--c-surface)",
                color: combinator === c ? "#fff" : "var(--c-text-2)",
                borderColor: combinator === c ? "var(--c-accent)" : "var(--c-border)",
              }}
            >
              {c.toUpperCase()}
            </button>
          ))}
        </div>

        <div style={{ flex: 1, overflowY: "auto", padding: "12px 20px", display: "flex", flexDirection: "column", gap: 8 }}>
          {rows.map((row) => (
            <div key={row.id} style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <select
                value={row.column}
                onChange={(e) => updateRow(row.id, { column: e.target.value })}
                style={{ ...SELECT_STYLE, flex: "0 0 160px" }}
              >
                {columns.map((col) => (
                  <option key={col.name} value={col.name}>{col.name}</option>
                ))}
              </select>

              <select
                value={row.operator}
                onChange={(e) => updateRow(row.id, { operator: e.target.value, value: "" })}
                style={{ ...SELECT_STYLE, flex: "0 0 120px" }}
              >
                {OPERATORS.map((op) => (
                  <option key={op.value} value={op.value}>{op.label}</option>
                ))}
              </select>

              {!NO_VALUE_OPS.has(row.operator) && (
                <input
                  value={row.value}
                  onChange={(e) => updateRow(row.id, { value: e.target.value })}
                  placeholder="value"
                  style={{ ...INPUT_STYLE, flex: 1 }}
                />
              )}

              {NO_VALUE_OPS.has(row.operator) && (
                <div style={{ flex: 1 }} />
              )}

              <button
                onClick={() => removeRow(row.id)}
                style={{
                  background: "none",
                  border: "1px solid var(--c-border)",
                  borderRadius: 4,
                  padding: "4px 8px",
                  fontSize: 12,
                  color: "#ef4444",
                  cursor: "pointer",
                }}
              >
                ×
              </button>
            </div>
          ))}

          <button
            onClick={addRow}
            style={{
              alignSelf: "flex-start",
              padding: "5px 12px",
              fontSize: 12,
              border: "1px dashed var(--c-border)",
              borderRadius: 4,
              background: "var(--c-surface)",
              color: "var(--c-text-2)",
              cursor: "pointer",
              marginTop: 4,
            }}
          >
            + Add condition
          </button>
        </div>

        <div
          style={{
            display: "flex",
            gap: 8,
            justifyContent: "flex-end",
            padding: "12px 20px",
            borderTop: "1px solid var(--c-border)",
            flexShrink: 0,
          }}
        >
          <button
            onClick={handleClear}
            style={{
              padding: "7px 14px",
              fontSize: 13,
              background: "var(--c-surface)",
              border: "1px solid var(--c-border)",
              borderRadius: 4,
              cursor: "pointer",
              color: "var(--c-text-2)",
            }}
          >
            Clear all
          </button>
          <button
            onClick={() => setShowFilterBuilder(false)}
            style={{
              padding: "7px 14px",
              fontSize: 13,
              background: "var(--c-surface)",
              border: "1px solid var(--c-border)",
              borderRadius: 4,
              cursor: "pointer",
              color: "var(--c-text-2)",
            }}
          >
            Cancel
          </button>
          <button
            onClick={handleApply}
            style={{
              padding: "7px 14px",
              fontSize: 13,
              background: "var(--c-accent)",
              color: "#fff",
              border: "none",
              borderRadius: 4,
              cursor: "pointer",
              fontWeight: 500,
            }}
          >
            Apply filters
          </button>
        </div>
      </div>
    </div>
  );
}
