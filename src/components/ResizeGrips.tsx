import { getCurrentWindow } from "@tauri-apps/api/window";
import { IS_LINUX } from "@/lib/platform";
import { useIsMaximized } from "@/lib/useMaximized";

/** Not exported by @tauri-apps/api (declared at window.d.ts:42, absent from the
 *  export list at :1822), so it is mirrored here. It is structurally checked
 *  against `Window.startResizeDragging(direction)` at the call site below, so if
 *  upstream ever changes the union this fails to compile rather than drifting. */
type Dir =
  | "East"
  | "North"
  | "NorthEast"
  | "NorthWest"
  | "South"
  | "SouthEast"
  | "SouthWest"
  | "West";

/**
 * Drag-to-resize edges for the undecorated Linux window (`decorations: false`
 * in tauri.linux.conf.json). A floating GNOME/KDE desktop draws no resize
 * border, so the app supplies one.
 *
 * Five hit targets, not eight: the left, right and bottom edges plus the two
 * bottom corners. The top edge and top corners are omitted because they would
 * sit over the header's drag region, and at the top right over
 * `WindowControls`, where a grip would eat the Close button's corner. `top-10`
 * matches the header's `h-10` so nothing overlaps it, and corners stack above
 * edges via `z-[1]`. None are drag regions (see `lib/dragRegion`), so a direct
 * hit resizes rather than moves.
 *
 * Accepted cost: a Wayland tiling WM reports no tiled state, so on Hyprland the
 * grips render permanently and inert, and the outer pixels of the nav rail and
 * composer swallow `pointerdown`.
 */
export function ResizeGrips() {
  const maximized = useIsMaximized();

  // Gated on IS_LINUX rather than the usual !IS_MACOS: Windows keeps a native
  // resize border under `decorations: false`, so grips there would double-handle
  // the drag on a platform this never intended to touch.
  if (!IS_LINUX) return null;
  if (maximized) return null; // a maximized window has no edge to drag

  return (
    <>
      <Grip dir="West" className="fixed left-0 top-10 bottom-0 w-1.5 cursor-w-resize" />
      <Grip dir="East" className="fixed right-0 top-10 bottom-0 w-1.5 cursor-e-resize" />
      <Grip dir="South" className="fixed bottom-0 left-0 right-0 h-1.5 cursor-s-resize" />
      <Grip
        dir="SouthWest"
        className="fixed bottom-0 left-0 h-3 w-3 z-[1] cursor-sw-resize"
      />
      <Grip
        dir="SouthEast"
        className="fixed bottom-0 right-0 h-3 w-3 z-[1] cursor-se-resize"
      />
    </>
  );
}

function Grip({ dir, className }: { dir: Dir; className: string }) {
  return (
    <div
      aria-hidden="true"
      className={className}
      onPointerDown={() => void getCurrentWindow().startResizeDragging(dir)}
    />
  );
}
