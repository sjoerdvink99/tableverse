import { useState, useCallback } from "react";
import { useTableStore } from "../stores/table";
import { useViewStore } from "../stores/view";
import { useUiStore } from "../stores/ui";
import { DEFAULT_CELL_W, DEFAULT_CELL_H } from "../lib/viewport";
import type { SourceMeta } from "../lib/types";

interface NavBarState {
  inputValue: string;
  focused: boolean;
  error: boolean;
  suggestions: string[];
}

function parseNavExpression(
  input: string,
  source: SourceMeta,
  currentScrollX: number,
  currentScrollY: number,
  zoom: number
): { scrollX: number; scrollY: number } | null {
  const cellW = DEFAULT_CELL_W * zoom;
  const cellH = DEFAULT_CELL_H * zoom;
  const nRows = source.n_rows;
  const nCols = source.n_cols;
  const trimmed = input.trim();

  const currentRow = Math.round(currentScrollY / cellH);
  const currentCol = Math.round(currentScrollX / cellW);

  if (/^\+\d+$/.test(trimmed)) {
    const delta = parseInt(trimmed.slice(1), 10);
    const row = Math.min(nRows - 1, currentRow + delta);
    return { scrollX: currentScrollX, scrollY: row * cellH };
  }
  if (/^-\d+$/.test(trimmed)) {
    const delta = parseInt(trimmed.slice(1), 10);
    const row = Math.max(0, currentRow - delta);
    return { scrollX: currentScrollX, scrollY: row * cellH };
  }

  if (trimmed.includes(":")) {
    const [rowPart, colPart] = trimmed.split(":");
    let row = currentRow;
    let col = currentCol;

    if (rowPart && rowPart.trim() !== "") {
      const rp = rowPart.trim();
      if (rp.endsWith("%")) {
        const pct = parseFloat(rp);
        if (isNaN(pct)) return null;
        row = Math.floor((pct / 100) * nRows);
      } else {
        const n = parseInt(rp, 10);
        if (isNaN(n)) return null;
        row = n;
      }
    }

    if (colPart && colPart.trim() !== "") {
      const cp = colPart.trim();
      if (cp.endsWith("%")) {
        const pct = parseFloat(cp);
        if (isNaN(pct)) return null;
        col = Math.floor((pct / 100) * nCols);
      } else {
        const n = parseInt(cp, 10);
        if (!isNaN(n)) {
          col = n;
        } else {
          const idx = source.columns.findIndex((c) => c.name === cp);
          if (idx < 0) return null;
          col = idx;
        }
      }
    }

    row = Math.max(0, Math.min(nRows - 1, row));
    col = Math.max(0, Math.min(nCols - 1, col));
    return { scrollX: col * cellW, scrollY: row * cellH };
  }

  if (/^\d+%$/.test(trimmed)) {
    const pct = parseFloat(trimmed);
    const row = Math.floor((pct / 100) * nRows);
    return { scrollX: currentScrollX, scrollY: Math.max(0, Math.min(nRows - 1, row)) * cellH };
  }

  if (/^\d+$/.test(trimmed)) {
    const row = Math.max(0, Math.min(nRows - 1, parseInt(trimmed, 10)));
    return { scrollX: currentScrollX, scrollY: row * cellH };
  }

  const colIdx = source.columns.findIndex((c) => c.name === trimmed);
  if (colIdx >= 0) {
    return { scrollX: colIdx * cellW, scrollY: currentScrollY };
  }

  return null;
}

function getSuggestions(input: string, source: SourceMeta): string[] {
  const colonIdx = input.indexOf(":");
  if (colonIdx < 0) return [];
  const afterColon = input.slice(colonIdx + 1);
  if (/^\d/.test(afterColon) || afterColon === "") return [];
  const lower = afterColon.toLowerCase();
  return source.columns
    .filter((c) => c.name.toLowerCase().startsWith(lower))
    .slice(0, 8)
    .map((c) => c.name);
}

function formatDisplayValue(source: SourceMeta, scrollX: number, scrollY: number, zoom: number): string {
  const cellW = DEFAULT_CELL_W * zoom;
  const cellH = DEFAULT_CELL_H * zoom;
  const row = Math.round(scrollY / cellH);
  const col = Math.round(scrollX / cellW);
  const colName = source.columns[col]?.name ?? String(col);
  return `Row ${row.toLocaleString()} / ${source.n_rows.toLocaleString()}  ·  Col ${col}: ${colName}`;
}

export function useNavigationBar() {
  const [state, setState] = useState<NavBarState>({
    inputValue: "",
    focused: false,
    error: false,
    suggestions: [],
  });

  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);

  const onFocus = useCallback(() => {
    if (!source) return;
    const { zoom } = useUiStore.getState();
    setState((s) => ({
      ...s,
      focused: true,
      inputValue: formatDisplayValue(source, viewport.scrollX, viewport.scrollY, zoom),
      error: false,
    }));
  }, [source, viewport]);

  const onBlur = useCallback(() => {
    setState((s) => ({ ...s, focused: false, error: false, suggestions: [] }));
  }, []);

  const onChange = useCallback(
    (value: string) => {
      if (!source) return;
      const sug = getSuggestions(value, source);
      const { zoom } = useUiStore.getState();
      const result = parseNavExpression(value, source, viewport.scrollX, viewport.scrollY, zoom);
      setState((s) => ({
        ...s,
        inputValue: value,
        suggestions: sug,
        error: value.trim() !== "" && result === null,
      }));
    },
    [source, viewport]
  );

  const onCommit = useCallback(() => {
    if (!source) return;
    const { zoom } = useUiStore.getState();
    const result = parseNavExpression(state.inputValue, source, viewport.scrollX, viewport.scrollY, zoom);
    if (result) {
      setState((s) => ({ ...s, error: false }));
    } else {
      setState((s) => ({ ...s, error: true }));
    }
    return result;
  }, [source, viewport, state.inputValue]);

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent, onNavigate: (pos: { scrollX: number; scrollY: number }) => void, onFocusEnd: () => void) => {
      if (e.key === "Escape") {
        onFocusEnd();
        return;
      }
      if (e.key === "Enter") {
        const result = onCommit();
        if (result) {
          onNavigate(result);
          onFocusEnd();
        }
      }
    },
    [onCommit]
  );

  const displayValue = useCallback(() => {
    if (!source) return "";
    const { zoom } = useUiStore.getState();
    return formatDisplayValue(source, viewport.scrollX, viewport.scrollY, zoom);
  }, [source, viewport]);

  return { state, onFocus, onBlur, onChange, onCommit, onKeyDown, displayValue, getSuggestions: (val: string) => source ? getSuggestions(val, source) : [] };
}
