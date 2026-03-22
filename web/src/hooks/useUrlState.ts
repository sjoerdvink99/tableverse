import { useEffect, useRef } from "react";
import { useViewStore } from "../stores/view";
import { useTableStore } from "../stores/table";
import { useUiStore } from "../stores/ui";
import { encodeUrlState, decodeUrlState } from "../lib/urlState";
import { getSource } from "../lib/api";

export function useUrlState() {
  const sourceId = useViewStore((s) => s.sourceId);
  const ops = useViewStore((s) => s.ops);
  const viewHash = useViewStore((s) => s.viewHash);
  const scrollX = useTableStore((s) => s.viewport.scrollX);
  const scrollY = useTableStore((s) => s.viewport.scrollY);
  const zoom = useUiStore((s) => s.zoom);
  const setSourceId = useViewStore((s) => s.setSourceId);
  const setOps = useViewStore((s) => s.setOps);
  const setSource = useTableStore((s) => s.setSource);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const stored = decodeUrlState();
    if (stored.sourceId) {
      getSource(stored.sourceId)
        .then((source) => {
          setSource(source);
          setSourceId(source.id);
          if (stored.ops) {
            try {
              const parsed = JSON.parse(stored.ops) as Parameters<typeof setOps>[0];
              setOps(parsed);
            } catch {}
          }
        })
        .catch(() => {
          window.location.hash = "";
        });
    }
  }, []);

  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      if (!sourceId) return;
      const encoded = encodeUrlState({
        sourceId,
        ops: JSON.stringify(ops),
        sx: scrollX,
        sy: scrollY,
        zoom,
      });
      window.location.hash = encoded;
    }, 500);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [sourceId, viewHash, scrollX, scrollY, zoom]);
}
