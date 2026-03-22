import { useEffect } from "react";
import { useViewStore } from "../stores/view";
import { useTableStore } from "../stores/table";
import { fetchViewSchema } from "../lib/api";

const SCHEMA_AFFECTING_OPS = new Set(["select", "drop", "derive", "group_by"]);

export function useVirtualSchema() {
  const source = useTableStore((s) => s.source);
  const ops = useViewStore((s) => s.ops);
  const viewHash = useViewStore((s) => s.viewHash);
  const viewExpr = useViewStore((s) => s.buildViewExpr)();
  const setVirtualSchema = useViewStore((s) => s.setVirtualSchema);

  const hasSchemaOps = ops.some((op) => SCHEMA_AFFECTING_OPS.has(op.type));

  useEffect(() => {
    if (!source || !viewExpr || !hasSchemaOps) {
      setVirtualSchema(null);
      return;
    }

    fetchViewSchema(viewExpr)
      .then(setVirtualSchema)
      .catch(() => setVirtualSchema(null));
  }, [viewHash, hasSchemaOps, source]);
}
