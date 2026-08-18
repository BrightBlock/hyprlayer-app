import { getCurrentWindow } from "@tauri-apps/api/window";

import { IS_LINUX } from "./platform";

/**
 * Window dragging from the app's own title bar.
 *
 * Everywhere but Linux this is Tauri's built-in `data-tauri-drag-region`: its
 * injected script starts the drag on `mousedown`. That is correct on macOS
 * (`performWindowDragWithEvent` applies the system's own drag threshold) and on
 * Windows (the `WM_NCLBUTTONDOWN` move loop only moves once the pointer has
 * travelled), but on Linux `start_dragging` calls straight into
 * `begin_move_drag`, which takes a compositor pointer grab the instant the
 * button goes down. A plain click on the header then feels like the window has
 * been grabbed: the pointer is captured, the app stops seeing the click, and a
 * one-pixel wobble slides the window.
 *
 * So on Linux the attribute is not emitted at all — Tauri's listener never
 * matches — and `setupDragRegion` installs this equivalent that waits for the
 * pointer to travel {@link DRAG_THRESHOLD_PX} before asking for the move, the
 * way Chromium-based apps (Brave, VS Code) behave. Below the threshold the
 * press stays an ordinary click.
 */

/** Bare = only a direct hit on the element drags; `deep` = its subtree drags too. */
export type DragRegionMode = "self" | "deep";

const TAURI_ATTR = "data-tauri-drag-region";
/** Linux-only mirror of the Tauri attribute, matched by the listener below. */
const LINUX_ATTR = "data-drag-region";

/**
 * Spread onto a title-bar element to make it drag the window:
 * `<header {...dragRegion()}>`, `<span {...dragRegion("deep")}>`.
 */
export function dragRegion(
  mode: DragRegionMode = "self",
): Record<string, string> {
  const value = mode === "deep" ? "deep" : "";
  return { [IS_LINUX ? LINUX_ATTR : TAURI_ATTR]: value };
}

/**
 * How far the pointer must travel before the press becomes a window move.
 * 5px — GTK's own drag threshold is 8, which reads as sticky on a title bar;
 * anything under ~4 is inside the jitter of a firm click.
 */
const DRAG_THRESHOLD_PX = 5;

const CLICKABLE_TAGS = new Set([
  "A",
  "BUTTON",
  "INPUT",
  "SELECT",
  "TEXTAREA",
  "LABEL",
  "SUMMARY",
]);
const INTERACTIVE_ROLES = new Set([
  "button",
  "link",
  "menuitem",
  "tab",
  "checkbox",
  "radio",
  "switch",
  "option",
]);

function isClickable(el: HTMLElement): boolean {
  return (
    CLICKABLE_TAGS.has(el.tagName) ||
    (el.hasAttribute("contenteditable") &&
      el.getAttribute("contenteditable") !== "false") ||
    (el.hasAttribute("tabindex") && el.getAttribute("tabindex") !== "-1") ||
    INTERACTIVE_ROLES.has(el.getAttribute("role") ?? "")
  );
}

/**
 * Same walk as Tauri's `drag.js`, over our attribute: climb the composed path,
 * let a clickable element (a button, a link) veto the drag before any region
 * above it is reached, honour `false` as an explicit opt-out, and treat a bare
 * value as "direct hits only".
 */
export function isDragRegion(path: EventTarget[]): boolean {
  for (const el of path) {
    if (!(el instanceof HTMLElement)) continue;
    const attr = el.getAttribute(LINUX_ATTR);
    if (isClickable(el) && attr === null) return false;
    if (attr === null) continue;
    if (attr === "false") return false;
    if (attr === "deep") return true;
    if (attr === "" || attr === "true") return el === path[0];
  }
  return false;
}

/** Seam for tests; defaults to the real window. */
export interface DragRegionHost {
  startDragging: () => void;
  toggleMaximize: () => void;
}

const tauriHost: DragRegionHost = {
  startDragging: () => void getCurrentWindow().startDragging(),
  toggleMaximize: () => void getCurrentWindow().toggleMaximize(),
};

/**
 * Installs the threshold-based drag handler. Exported for tests; app code calls
 * {@link setupDragRegion}, which platform-gates this. Returns a teardown.
 */
export function installDragRegion(
  doc: Document = document,
  host: DragRegionHost = tauriHost,
): () => void {
  // Non-null only between a press on a drag region and the move that promotes
  // it to a window drag (or the release that leaves it a click).
  let origin: { x: number; y: number } | null = null;

  const onMouseDown = (e: MouseEvent) => {
    origin = null;
    if (e.button !== 0) return;
    // detail 1 or 2 only: a triple click is neither a drag nor a maximize.
    if (e.detail !== 1 && e.detail !== 2) return;
    if (!isDragRegion(e.composedPath())) return;
    // Suppresses the text cursor and the native image drag, as Tauri's own
    // handler does. The click event still fires, which is the point.
    e.preventDefault();
    if (e.detail === 2) {
      host.toggleMaximize();
      return;
    }
    origin = { x: e.clientX, y: e.clientY };
  };

  const onMouseMove = (e: MouseEvent) => {
    if (!origin) return;
    if (
      Math.hypot(e.clientX - origin.x, e.clientY - origin.y) < DRAG_THRESHOLD_PX
    )
      return;
    // Clear first: once the compositor takes the grab this webview stops seeing
    // the pointer, so there is no mouseup coming to clean up after us.
    origin = null;
    host.startDragging();
  };

  const onMouseUp = () => {
    origin = null;
  };

  doc.addEventListener("mousedown", onMouseDown);
  doc.addEventListener("mousemove", onMouseMove);
  doc.addEventListener("mouseup", onMouseUp);
  return () => {
    doc.removeEventListener("mousedown", onMouseDown);
    doc.removeEventListener("mousemove", onMouseMove);
    doc.removeEventListener("mouseup", onMouseUp);
  };
}

/** Wired from `main.tsx`. No-op off Linux, where Tauri's own handler is right. */
export function setupDragRegion(): void {
  if (!IS_LINUX) return;
  installDragRegion();
}
