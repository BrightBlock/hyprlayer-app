import type { KeyboardEvent, MouseEvent } from "react";

export const LIST_ROW = { "data-list-row": "" } as const;

const isCurrent = (row: HTMLElement) => row.getAttribute("aria-current") === "true";

function activate(row: HTMLElement) {
  // Before the click, whose onListNavClick focus would skip the scroll.
  row.focus();
  if (!isCurrent(row)) row.click();
}

// Puts the arrow keys on a list: its current row, else its first, marked like keyboard focus.
export function focusList(root: HTMLElement) {
  const rows = Array.from(root.querySelectorAll<HTMLElement>("[data-list-row]"));
  (rows.find(isCurrent) ?? rows[0])?.focus({ focusVisible: true });
}

// WKWebView never focuses a clicked button, and the keys below need the row focused.
// Not on mousedown: WebKit clears focus after it.
export function onListNavClick(e: MouseEvent<HTMLElement>) {
  (e.target as Element).closest<HTMLElement>("[data-list-row]")?.focus({ preventScroll: true });
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
  // With nothing open yet, the first arrow opens the focused row instead of passing it.
  const step = e.key === "ArrowDown" || e.key === "ArrowUp";
  const to = step && !rows.some(isCurrent) ? i : next[e.key];
  activate(rows[Math.max(0, Math.min(to, rows.length - 1))]);
}
