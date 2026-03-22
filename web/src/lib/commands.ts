export type Command = {
  id: string;
  label: string;
  category: "navigation" | "filter" | "view" | "export" | "source";
  shortcut?: string;
  action: () => void;
};
