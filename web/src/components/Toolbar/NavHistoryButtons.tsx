import { useUiStore } from "../../stores/ui";
import { useNavigation } from "../../hooks/useNavigation";

export function NavHistoryButtons() {
  const navHistory = useUiStore((s) => s.navHistory);
  const navHistoryIdx = useUiStore((s) => s.navHistoryIdx);
  const navHistoryBack = useUiStore((s) => s.navHistoryBack);
  const navHistoryForward = useUiStore((s) => s.navHistoryForward);
  const { navigateTo } = useNavigation();

  const canBack = navHistoryIdx > 0;
  const canForward = navHistoryIdx < navHistory.length - 1;

  const handleBack = () => {
    const pos = navHistoryBack();
    if (pos) navigateTo(pos.scrollX, pos.scrollY, false);
  };

  const handleForward = () => {
    const pos = navHistoryForward();
    if (pos) navigateTo(pos.scrollX, pos.scrollY, false);
  };

  return (
    <div style={{ display: "flex", gap: 2 }}>
      <button
        className="tv-btn tv-btn-ghost"
        onClick={handleBack}
        disabled={!canBack}
        title="Back (Alt+←)"
        style={{ opacity: canBack ? 1 : 0.35 }}
      >
        ←
      </button>
      <button
        className="tv-btn tv-btn-ghost"
        onClick={handleForward}
        disabled={!canForward}
        title="Forward (Alt+→)"
        style={{ opacity: canForward ? 1 : 0.35 }}
      >
        →
      </button>
    </div>
  );
}
