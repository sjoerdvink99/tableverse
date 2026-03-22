import { useEffect, useRef } from "react";
import { useTableStore } from "../stores/table";
import { useStatsStore } from "../stores/stats";
import { fetchCorrelations } from "../lib/api";

export function useCorrelations(): void {
  const source = useTableStore((s) => s.source);
  const setCorrelations = useStatsStore((s) => s.setCorrelations);
  const correlations = useStatsStore((s) => s.correlations);
  const fetchingRef = useRef<string | null>(null);

  useEffect(() => {
    if (!source) return;
    if (correlations?.sourceId === source.id) return;
    if (fetchingRef.current === source.id) return;

    fetchingRef.current = source.id;
    fetchCorrelations(source.id)
      .then((matrix) => {
        setCorrelations({ sourceId: source.id, matrix });
        fetchingRef.current = null;
      })
      .catch(() => {
        fetchingRef.current = null;
      });
  }, [source?.id, correlations, setCorrelations]);
}
