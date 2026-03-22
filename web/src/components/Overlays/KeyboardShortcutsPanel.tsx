import { useEffect } from "react";
import { useUiStore } from "../../stores/ui";

interface ShortcutEntry {
  label: string;
  keys: string[][];
}

interface ShortcutGroup {
  title: string;
  entries: ShortcutEntry[];
}

const GROUPS: ShortcutGroup[] = [
  {
    title: "Navigation",
    entries: [
      { label: "Move one cell", keys: [["↑"], ["↓"], ["←"], ["→"]] },
      { label: "Jump 10 cells", keys: [["Ctrl"], ["↑ ↓ ← →"]] },
      { label: "Scroll page", keys: [["PgUp"], ["PgDn"]] },
      { label: "Scroll 10 pages", keys: [["Shift"], ["PgUp"], ["PgDn"]] },
      { label: "Jump to row start", keys: [["Home"]] },
      { label: "Jump to row end", keys: [["End"]] },
      { label: "Jump to first cell", keys: [["Ctrl"], ["Home"]] },
      { label: "Jump to last cell", keys: [["Ctrl"], ["End"]] },
      { label: "Jump to row…", keys: [["Ctrl"], ["G"]] },
      { label: "Next landmark", keys: [["Ctrl"], ["↓"]] },
      { label: "Prev landmark", keys: [["Ctrl"], ["↑"]] },
      { label: "History back", keys: [["Alt"], ["←"]] },
      { label: "History forward", keys: [["Alt"], ["→"]] },
    ],
  },
  {
    title: "Scroll & Zoom",
    entries: [
      { label: "Scroll vertically", keys: [["wheel"]] },
      { label: "Scroll horizontally", keys: [["Shift"], ["wheel"]] },
      { label: "Pinch zoom", keys: [["Ctrl"], ["wheel"]] },
      { label: "Zoom in", keys: [["Ctrl"], ["+"]] },
      { label: "Zoom out", keys: [["Ctrl"], ["−"]] },
      { label: "Reset zoom", keys: [["click 100%"]] },
    ],
  },
  {
    title: "Panels",
    entries: [
      { label: "Command palette", keys: [["Ctrl"], ["P"]] },
      { label: "Filter builder", keys: [["Ctrl"], ["Shift"], ["F"]] },
      { label: "Columns panel", keys: [["Ctrl"], ["Shift"], ["C"]] },
      { label: "Export", keys: [["Ctrl"], ["E"]] },
      { label: "Minimap", keys: [["Ctrl"], ["M"]] },
      { label: "Bookmarks panel", keys: [["Ctrl"], ["Shift"], ["B"]] },
      { label: "Landmarks panel", keys: [["Ctrl"], ["Shift"], ["L"]] },
      { label: "Keyboard shortcuts", keys: [["?"]] },
    ],
  },
  {
    title: "Bookmarks",
    entries: [
      { label: "Add bookmark", keys: [["Ctrl"], ["B"]] },
      { label: "Jump to bookmark 1–8", keys: [["Ctrl"], ["1–8"]] },
    ],
  },
];

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd
      style={{
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        padding: "1px 6px",
        fontSize: 10.5,
        fontFamily: "inherit",
        fontWeight: 500,
        color: "var(--c-text-2)",
        background: "var(--c-surface-2)",
        border: "1px solid var(--c-border)",
        borderBottom: "2px solid var(--c-border-hover)",
        borderRadius: 4,
        lineHeight: 1.6,
        whiteSpace: "nowrap",
      }}
    >
      {children}
    </kbd>
  );
}

function ShortcutRow({ label, keys }: ShortcutEntry) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: "3.5px 14px",
        gap: 8,
      }}
    >
      <span style={{ fontSize: 12.5, color: "var(--c-text-2)", flex: 1, minWidth: 0, lineHeight: 1.4 }}>
        {label}
      </span>
      <div style={{ display: "flex", gap: 3, alignItems: "center", flexShrink: 0 }}>
        {keys.map((group, gi) => (
          <span key={gi} style={{ display: "flex", alignItems: "center", gap: 3 }}>
            {gi > 0 && (
              <span style={{ fontSize: 10, color: "var(--c-text-3)", marginRight: 1 }}>+</span>
            )}
            {group.map((k, ki) => {
              if (k === "wheel" || k.startsWith("click ")) {
                return (
                  <span key={ki} style={{ fontSize: 11, color: "var(--c-text-3)", fontStyle: "italic" }}>
                    {k}
                  </span>
                );
              }
              return <Kbd key={ki}>{k}</Kbd>;
            })}
          </span>
        ))}
      </div>
    </div>
  );
}

export function KeyboardShortcutsPanel() {
  const showKeyboardShortcuts = useUiStore((s) => s.showKeyboardShortcuts);
  const setShowKeyboardShortcuts = useUiStore((s) => s.setShowKeyboardShortcuts);

  useEffect(() => {
    if (!showKeyboardShortcuts) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape" || e.key === "?") {
        e.preventDefault();
        setShowKeyboardShortcuts(false);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [showKeyboardShortcuts, setShowKeyboardShortcuts]);

  if (!showKeyboardShortcuts) return null;

  return (
    <>
      <div
        onClick={() => setShowKeyboardShortcuts(false)}
        style={{
          position: "fixed",
          inset: 0,
          zIndex: 900,
          background: "rgba(0,0,0,0.25)",
          backdropFilter: "blur(1px)",
        }}
      />
      <div
        style={{
          position: "fixed",
          top: 52,
          right: 10,
          width: 400,
          maxHeight: "calc(100vh - 72px)",
          background: "var(--c-bg)",
          border: "1px solid var(--c-border)",
          borderRadius: 10,
          boxShadow: "0 20px 60px rgba(0,0,0,0.22), 0 4px 16px rgba(0,0,0,0.1)",
          zIndex: 901,
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
            padding: "11px 14px",
            borderBottom: "1px solid var(--c-border)",
            flexShrink: 0,
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: 7 }}>
            <span style={{ fontSize: 13.5, fontWeight: 600, color: "var(--c-text)", letterSpacing: "-0.02em" }}>
              Keyboard Shortcuts
            </span>
          </div>
          <button
            onClick={() => setShowKeyboardShortcuts(false)}
            style={{
              background: "none",
              border: "none",
              cursor: "pointer",
              color: "var(--c-text-3)",
              fontSize: 20,
              lineHeight: 1,
              padding: "0 2px",
              display: "flex",
              alignItems: "center",
              borderRadius: 4,
              transition: "color 100ms",
            }}
            onMouseEnter={(e) => ((e.target as HTMLButtonElement).style.color = "var(--c-text)")}
            onMouseLeave={(e) => ((e.target as HTMLButtonElement).style.color = "var(--c-text-3)")}
          >
            ×
          </button>
        </div>

        <div style={{ overflowY: "auto", flex: 1, paddingBottom: 8 }}>
          {GROUPS.map((group, gi) => (
            <div key={group.title} style={{ marginTop: gi === 0 ? 8 : 4 }}>
              <div
                style={{
                  padding: "4px 14px 5px",
                  fontSize: 10,
                  fontWeight: 700,
                  letterSpacing: "0.08em",
                  textTransform: "uppercase",
                  color: "var(--c-text-3)",
                  marginBottom: 1,
                }}
              >
                {group.title}
              </div>
              {group.entries.map((entry) => (
                <ShortcutRow key={entry.label} label={entry.label} keys={entry.keys} />
              ))}
            </div>
          ))}
        </div>

        <div
          style={{
            padding: "7px 14px",
            borderTop: "1px solid var(--c-border)",
            display: "flex",
            alignItems: "center",
            gap: 4,
            flexShrink: 0,
          }}
        >
          <span style={{ fontSize: 11, color: "var(--c-text-3)" }}>Press</span>
          <Kbd>Esc</Kbd>
          <span style={{ fontSize: 11, color: "var(--c-text-3)" }}>or</span>
          <Kbd>?</Kbd>
          <span style={{ fontSize: 11, color: "var(--c-text-3)" }}>to close</span>
        </div>
      </div>
    </>
  );
}
