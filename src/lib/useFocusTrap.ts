import { useEffect, type KeyboardEvent, type RefObject } from "react";

const TABBABLE =
  "a[href], button:not([disabled]), input:not([disabled]):not([type='hidden']), " +
  "select:not([disabled]), textarea:not([disabled]), [tabindex]";

function tabbables(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(TABBABLE)).filter(
    (el) => el.tabIndex >= 0 && (el.checkVisibility?.() ?? true),
  );
}

type FocusTrapOptions = {
  initialFocusRef?: RefObject<HTMLElement | null>;
  restoreFocusRef?: RefObject<HTMLElement | null>;
};

/**
 * Moves focus into `panelRef` while `active`, cycles Tab inside it, and hands
 * focus back on release. Tab is cycled by hand because WKWebView skips buttons
 * unless macOS keyboard navigation is on.
 */
export function useFocusTrap(
  panelRef: RefObject<HTMLElement | null>,
  active: boolean,
  { initialFocusRef, restoreFocusRef }: FocusTrapOptions = {},
): (e: KeyboardEvent<HTMLElement>) => void {
  useEffect(() => {
    if (!active) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const restore = restoreFocusRef?.current ?? previous;
    const panel = panelRef.current;
    if (panel) {
      const target = initialFocusRef?.current ?? tabbables(panel)[0] ?? panel;
      target.focus({ preventScroll: true });
    }
    return () => {
      if (restore?.isConnected) restore.focus({ preventScroll: true });
    };
    // The refs are stable; only opening and closing moves focus.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  return (e) => {
    if (!active || e.key !== "Tab") return;
    const panel = panelRef.current;
    // Focus inside a nested portal (a dropdown's listbox) keeps its own keys.
    if (!panel || !panel.contains(document.activeElement)) return;
    e.preventDefault();
    const items = tabbables(panel);
    if (items.length === 0) return;
    const at = items.indexOf(document.activeElement as HTMLElement);
    const next = e.shiftKey
      ? items[at <= 0 ? items.length - 1 : at - 1]
      : items[at === -1 || at === items.length - 1 ? 0 : at + 1];
    next.focus();
  };
}
