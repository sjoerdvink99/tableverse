import { useRef, useState } from "react";
import { useTableStore } from "../../stores/table";
import { useViewStore } from "../../stores/view";

function SearchIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 13 13" fill="none">
      <circle cx="5.5" cy="5.5" r="3.75" stroke="currentColor" strokeWidth="1.4" />
      <line x1="8.5" y1="8.5" x2="11.5" y2="11.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  );
}

export function SearchBar() {
  const source = useTableStore((s) => s.source);
  const virtualSchema = useViewStore((s) => s.virtualSchema);
  const [query, setQuery] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  const textColumnNames = (): string[] => {
    const schema = virtualSchema ?? source?.columns ?? [];
    return schema
      .filter((c) => {
        const dt = c.data_type.toLowerCase();
        return dt === "utf8" || dt === "text" || dt === "string" || dt.includes("varchar") || dt.includes("char");
      })
      .map((c) => c.name);
  };

  const handleChange = (value: string) => {
    setQuery(value);
    if (!value.trim()) {
      useViewStore.getState().clearSearch();
    } else {
      const cols = textColumnNames();
      if (cols.length > 0) {
        useViewStore.getState().setSearch(value.trim(), cols);
      }
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      setQuery("");
      useViewStore.getState().clearSearch();
    }
  };

  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
      <div className="tv-search-wrap">
        <span className="tv-search-icon">
          <SearchIcon />
        </span>
        <input
          ref={inputRef}
          value={query}
          onChange={(e) => handleChange(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Search…"
          className="tv-search-input"
          disabled={!source}
        />
      </div>
    </div>
  );
}
