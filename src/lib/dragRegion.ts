import { getCurrentWindow } from "@tauri-apps/api/window";

import { IS_LINUX } from "./platform";


type DragRegionMode = "self" | "deep";

const TAURI_ATTR = "data-tauri-drag-region";
const LINUX_ATTR = "data-drag-region";

export function dragRegion(
  mode: DragRegionMode = "self",
): Record<string, string> {
  const value = mode === "deep" ? "deep" : "";
  return { [IS_LINUX ? LINUX_ATTR : TAURI_ATTR]: value };
}

// Tauri grabs immediately on Linux, so ignore normal click jitter.
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

interface DragRegionHost {
  startDragging: () => void;
  toggleMaximize: () => void;
}

const tauriHost: DragRegionHost = {
  startDragging: () => void getCurrentWindow().startDragging(),
  toggleMaximize: () => void getCurrentWindow().toggleMaximize(),
};

export function installDragRegion(
  doc: Document = document,
  host: DragRegionHost = tauriHost,
): () => void {
  let origin: { x: number; y: number } | null = null;

  const onMouseDown = (e: MouseEvent) => {
    origin = null;
    if (e.button !== 0) return;
    if (e.detail !== 1 && e.detail !== 2) return;
    if (!isDragRegion(e.composedPath())) return;
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
    // The compositor grab prevents a later mouseup.
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

export function setupDragRegion(): void {
  if (!IS_LINUX) return;
  installDragRegion();
}
