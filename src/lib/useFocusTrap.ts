import { useEffect, type KeyboardEvent, type RefObject } from "react";

const TABBABLE =
  "a[href], button:not([disabled]), input:not([disabled]):not([type='hidden']), " +
  "select:not([disabled]), textarea:not([disabled]), [tabindex]";

// For engines without `checkVisibility` (older WebKit): focusing an element
// that is not displayed fails silently and would strand Tab.
function displayed(el: HTMLElement, root: HTMLElement): boolean {
  for (let node: HTMLElement | null = el; node && node !== root; node = node.parentElement) {
    const style = getComputedStyle(node);
    if (style.display === "none" || style.visibility === "hidden") return false;
  }
  return true;
}

function tabbables(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(TABBABLE)).filter(
    (el) =>
      el.tabIndex >= 0 &&
      !el.closest("[inert]") &&
      (el.checkVisibility?.() ?? displayed(el, root)),
  );
}

type FocusTrapOptions = {
  initialFocusRef?: RefObject<HTMLElement | null>;
  restoreFocusRef?: RefObject<HTMLElement | null>;
};

/**
 * Moves focus into `panelRef` while `active`, cycles Tab inside it, and hands
 * focus back on release unless a pointer was last used. Tab is cycled by hand
 * because WKWebView skips buttons unless macOS keyboard navigation is on.
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
    // Focus handed back after a click would show WebKit's focus ring, inherited
    // from a text field that had focus; only a keyboard user needs it back.
    let byPointer = false;
    const onPointer = () => {
      byPointer = true;
    };
    const onKey = () => {
      byPointer = false;
    };
    document.addEventListener("pointerdown", onPointer, true);
    document.addEventListener("keydown", onKey, true);
    const panel = panelRef.current;
    if (panel && (initialFocusRef?.current || !panel.contains(document.activeElement))) {
      const target = initialFocusRef?.current ?? tabbables(panel)[0] ?? panel;
      target.focus({ preventScroll: true });
    }
    return () => {
      document.removeEventListener("pointerdown", onPointer, true);
      document.removeEventListener("keydown", onKey, true);
      if (!byPointer && restore?.isConnected) restore.focus({ preventScroll: true });
    };
    // The refs are stable; only opening and closing moves focus.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  return (e) => {
    if (!active || e.key !== "Tab" || e.defaultPrevented) return;
    const panel = panelRef.current;
    // Focus inside a nested portal (a dropdown's listbox) keeps its own keys.
    if (!panel || !panel.contains(document.activeElement)) return;
    e.preventDefault();
    // Shift+Tab is also a window-level shortcut (ws.cycleStage).
    e.stopPropagation();
    const items = tabbables(panel);
    if (items.length === 0) return;
    const at = items.indexOf(document.activeElement as HTMLElement);
    const next = e.shiftKey
      ? items[at <= 0 ? items.length - 1 : at - 1]
      : items[at === -1 || at === items.length - 1 ? 0 : at + 1];
    next.focus();
  };
}
