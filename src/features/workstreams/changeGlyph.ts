export const GLYPH: Record<string, { ch: string; cls: string; label: string }> = {
  added: { ch: "+", cls: "text-ok", label: "added" },
  untracked: { ch: "+", cls: "text-ok", label: "untracked" },
  modified: { ch: "M", cls: "text-accent", label: "modified" },
  deleted: { ch: "−", cls: "text-bad", label: "deleted" },
  renamed: { ch: "R", cls: "text-warn", label: "renamed" },
  typechange: { ch: "T", cls: "text-warn", label: "typechange" },
  conflict: { ch: "!", cls: "text-bad", label: "conflict" },
};
