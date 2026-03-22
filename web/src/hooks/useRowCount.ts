import { useEffect, useRef } from "react";
import { useViewStore } from "../stores/view";
import { useTableStore } from "../stores/table";
import { fetchViewCount } from "../lib/api";

const DEBOUNCE_MS = 300;

export function useRowCount() {
  const source = useTableStore((s) => s.source);
  const viewExpr = useViewStore((s) => s.buildViewExpr)();
  const viewHash = useViewStore((s) => s.viewHash);
  const setVirtualRowCount = useViewStore((s) => s.setVirtualRowCount);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!source || !viewExpr) {
      setVirtualRowCount(null);
      return;
    }

    if (viewExpr.ops.length === 0) {
      setVirtualRowCount(source.n_rows);
      return;
    }

    const COUNT_CHANGING_TYPES = new Set(["filter", "deduplicate", "sample", "group_by", "limit"]);
    const changesCount = viewExpr.ops.some((op) => COUNT_CHANGING_TYPES.has(op.type));
    if (!changesCount) {
      setVirtualRowCount(source.n_rows);
      return;
    }

    if (timerRef.current) clearTimeout(timerRef.current);

    timerRef.current = setTimeout(() => {
      fetchViewCount(viewExpr)
        .then(setVirtualRowCount)
        .catch(() => setVirtualRowCount(source.n_rows));
    }, DEBOUNCE_MS);

    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [viewHash, source]);
}
