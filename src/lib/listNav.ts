import type { KeyboardEvent } from "react";

export const LIST_ROW = { "data-list-row": "" } as const;

const isCurrent = (row: HTMLElement) => row.getAttribute("aria-current") === "true";

function activate(row: HTMLElement) {
  if (!isCurrent(row)) row.click();
  row.focus();
}

// Mail-style list keys: the arrows move the selection, the detail pane follows it.
export function onListNavKeyDown(e: KeyboardEvent<HTMLElement>) {
  if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
  const root = e.currentTarget;
  const target = e.target as HTMLElement;
  // React bubbles portal events (menus, popovers) through here too.
  if (!root.contains(target)) return;
  const rows = Array.from(root.querySelectorAll<HTMLElement>("[data-list-row]"));
  if (rows.length === 0) return;

  const from = target.closest<HTMLElement>("[data-list-row]");
  if (!from) {
    if (e.key !== "ArrowDown" || !(target instanceof HTMLInputElement)) return;
    e.preventDefault();
    const current = rows.find(isCurrent);
    if (current) current.focus();
    else activate(rows[0]);
    return;
  }

  const i = rows.indexOf(from);
  const page = Math.max(1, Math.floor(root.clientHeight / Math.max(from.offsetHeight, 1)) - 1);
  const next: Record<string, number> = {
    ArrowDown: i + 1,
    ArrowUp: i - 1,
    PageDown: i + page,
    PageUp: i - page,
    Home: 0,
    End: rows.length - 1,
  };
  if (!(e.key in next)) return;
  e.preventDefault();
  activate(rows[Math.max(0, Math.min(next[e.key], rows.length - 1))]);
}
