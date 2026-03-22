import { useRef } from "react";
import { useUiStore } from "../../stores/ui";
import { useBookmarkStore, BOOKMARK_COLORS } from "../../stores/bookmarkStore";
import { useNavigation } from "../../hooks/useNavigation";

export function BookmarkPanel() {
  const showBookmarkPanel = useUiStore((s) => s.showBookmarkPanel);
  const setShowBookmarkPanel = useUiStore((s) => s.setShowBookmarkPanel);
  const bookmarks = useBookmarkStore((s) => s.bookmarks);
  const removeBookmark = useBookmarkStore((s) => s.removeBookmark);
  const updateBookmark = useBookmarkStore((s) => s.updateBookmark);
  const { teleportTo } = useNavigation();

  if (!showBookmarkPanel) return null;

  return (
    <div
      style={{
        position: "fixed",
        top: 80,
        right: 124,
        width: 300,
        maxHeight: 420,
        background: "var(--c-bg)",
        border: "1px solid var(--c-border)",
        borderRadius: 6,
        boxShadow: "0 8px 24px rgba(0,0,0,0.14)",
        zIndex: 500,
        display: "flex",
        flexDirection: "column",
        overflow: "hidden",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          padding: "8px 12px",
          borderBottom: "1px solid var(--c-border)",
          flexShrink: 0,
        }}
      >
        <span style={{ fontSize: 13, fontWeight: 600, color: "var(--c-text)" }}>Bookmarks</span>
        <button
          onClick={() => setShowBookmarkPanel(false)}
          style={{ background: "none", border: "none", cursor: "pointer", color: "var(--c-text-3)", fontSize: 16, lineHeight: 1, padding: 0 }}
        >
          ×
        </button>
      </div>
      <div style={{ overflowY: "auto", flex: 1 }}>
        {bookmarks.length === 0 && (
          <div style={{ padding: 16, fontSize: 12, color: "var(--c-text-3)", textAlign: "center" }}>
            No bookmarks yet. Press Ctrl+B to add one.
          </div>
        )}
        {bookmarks.map((bm) => (
          <BookmarkRow
            key={bm.id}
            label={bm.label}
            color={bm.color}
            onJump={() => {
              teleportTo(bm.scrollX, bm.scrollY);
              setShowBookmarkPanel(false);
            }}
            onRemove={() => removeBookmark(bm.id)}
            onLabelChange={(label) => updateBookmark(bm.id, { label })}
          />
        ))}
      </div>
      <div
        style={{
          padding: "6px 12px",
          borderTop: "1px solid var(--c-border)",
          fontSize: 11,
          color: "var(--c-text-3)",
          flexShrink: 0,
        }}
      >
        Press Ctrl+1–8 to jump directly
      </div>
    </div>
  );
}

interface BookmarkRowProps {
  label: string;
  color: string;
  onJump: () => void;
  onRemove: () => void;
  onLabelChange: (label: string) => void;
}

function BookmarkRow({ label, color, onJump, onRemove, onLabelChange }: BookmarkRowProps) {
  const inputRef = useRef<HTMLInputElement>(null);

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "6px 12px",
        borderBottom: "1px solid var(--c-border)",
      }}
    >
      <div style={{ width: 10, height: 10, borderRadius: 2, background: color, flexShrink: 0 }} />
      <input
        ref={inputRef}
        type="text"
        defaultValue={label}
        onBlur={(e) => onLabelChange(e.target.value)}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") inputRef.current?.blur();
        }}
        style={{
          flex: 1,
          fontSize: 12,
          border: "none",
          background: "transparent",
          color: "var(--c-text)",
          outline: "none",
          minWidth: 0,
        }}
      />
      <button
        onClick={onJump}
        title="Jump to bookmark"
        style={{ background: "none", border: "none", cursor: "pointer", color: "var(--c-text-3)", fontSize: 12, padding: "0 2px" }}
      >
        →
      </button>
      <button
        onClick={onRemove}
        title="Remove bookmark"
        style={{ background: "none", border: "none", cursor: "pointer", color: "var(--c-text-3)", fontSize: 14, padding: "0 2px" }}
      >
        ×
      </button>
    </div>
  );
}
