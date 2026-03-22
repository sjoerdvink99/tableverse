import { useRef, useState } from "react";
import { useNavigationBar } from "../../hooks/useNavigationBar";
import { useNavigation } from "../../hooks/useNavigation";
import { useTableStore } from "../../stores/table";

export function NavigationBar() {
  const source = useTableStore((s) => s.source);
  const inputRef = useRef<HTMLInputElement>(null);
  const [suggestionIdx, setSuggestionIdx] = useState(-1);
  const { state, onFocus, onBlur, onChange, onKeyDown, displayValue, getSuggestions } = useNavigationBar();
  const { teleportTo } = useNavigation();

  if (!source) return null;

  const handleFocus = () => {
    onFocus();
    setTimeout(() => inputRef.current?.select(), 0);
  };

  const handleBlur = () => {
    onBlur();
    setSuggestionIdx(-1);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    const sug = getSuggestions(state.inputValue);
    if (e.key === "ArrowDown" && sug.length > 0) {
      e.preventDefault();
      setSuggestionIdx((i) => Math.min(sug.length - 1, i + 1));
      return;
    }
    if (e.key === "ArrowUp" && sug.length > 0) {
      e.preventDefault();
      setSuggestionIdx((i) => Math.max(-1, i - 1));
      return;
    }

    if (e.key === "Enter" && suggestionIdx >= 0 && sug[suggestionIdx]) {
      const colonIdx = state.inputValue.indexOf(":");
      const prefix = colonIdx >= 0 ? state.inputValue.slice(0, colonIdx + 1) : ":";
      onChange(prefix + sug[suggestionIdx]);
      setSuggestionIdx(-1);
      return;
    }

    onKeyDown(
      e,
      (pos) => teleportTo(pos.scrollX, pos.scrollY),
      () => inputRef.current?.blur()
    );
  };

  const suggestions = getSuggestions(state.inputValue);

  return (
    <div style={{ position: "relative" }}>
      <input
        ref={inputRef}
        type="text"
        value={state.focused ? state.inputValue : displayValue()}
        onFocus={handleFocus}
        onBlur={handleBlur}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={handleKeyDown}
        style={{
          width: 220,
          fontFamily: "monospace",
          fontSize: 12,
          padding: "3px 7px",
          border: `1px solid ${state.error ? "#ef4444" : "var(--c-border)"}`,
          borderRadius: 4,
          background: "var(--c-bg)",
          color: "var(--c-text)",
          outline: state.error ? "2px solid #ef4444" : "none",
          boxSizing: "border-box",
        }}
        placeholder="Row or row:col"
        title="Navigate: row, row:col, row%, +offset, :colName"
      />
      {state.focused && suggestions.length > 0 && (
        <ul
          style={{
            position: "absolute",
            top: "100%",
            left: 0,
            width: 220,
            margin: 0,
            padding: 0,
            listStyle: "none",
            background: "var(--c-bg)",
            border: "1px solid var(--c-border)",
            borderRadius: 4,
            boxShadow: "0 4px 12px rgba(0,0,0,0.12)",
            zIndex: 1000,
            maxHeight: 200,
            overflowY: "auto",
          }}
        >
          {suggestions.map((s, i) => (
            <li
              key={s}
              onMouseDown={(e) => {
                e.preventDefault();
                const colonIdx = state.inputValue.indexOf(":");
                const prefix = colonIdx >= 0 ? state.inputValue.slice(0, colonIdx + 1) : ":";
                onChange(prefix + s);
                setSuggestionIdx(-1);
              }}
              style={{
                padding: "4px 8px",
                fontSize: 12,
                fontFamily: "monospace",
                cursor: "pointer",
                background: i === suggestionIdx ? "var(--c-hover)" : "transparent",
                color: "var(--c-text)",
              }}
            >
              {s}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
