import { create } from "zustand";
import { persist } from "zustand/middleware";
import { categoricalColor, djb2 } from "../lib/color-scales";

export type Bookmark = {
  id: string;
  scrollX: number;
  scrollY: number;
  label: string;
  color: string;
};

type BookmarkStore = {
  bookmarks: Bookmark[];
  addBookmark: (b: Omit<Bookmark, "id">) => void;
  removeBookmark: (id: string) => void;
  updateBookmark: (id: string, changes: Partial<Pick<Bookmark, "label" | "color">>) => void;
};

function toHex(r: number, g: number, b: number): string {
  return `#${r.toString(16).padStart(2, "0")}${g.toString(16).padStart(2, "0")}${b.toString(16).padStart(2, "0")}`;
}

export const BOOKMARK_COLORS: string[] = Array.from({ length: 8 }, (_, i) => {
  const [r, g, b] = categoricalColor(djb2(`bm${i}`));
  return toHex(r, g, b);
});

let _idCounter = 0;

export const useBookmarkStore = create<BookmarkStore>()(
  persist(
    (set) => ({
      bookmarks: [],

      addBookmark: (b) =>
        set((s) => ({
          bookmarks: [...s.bookmarks, { ...b, id: `bm-${Date.now()}-${_idCounter++}` }],
        })),

      removeBookmark: (id) =>
        set((s) => ({ bookmarks: s.bookmarks.filter((b) => b.id !== id) })),

      updateBookmark: (id, changes) =>
        set((s) => ({
          bookmarks: s.bookmarks.map((b) => (b.id === id ? { ...b, ...changes } : b)),
        })),
    }),
    { name: "tv-bookmarks" }
  )
);
