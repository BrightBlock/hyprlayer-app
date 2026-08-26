import { getCurrentWindow } from "@tauri-apps/api/window";
import { IS_LINUX } from "@/lib/platform";
import { useIsMaximized } from "@/lib/useMaximized";

type Dir =
  | "East"
  | "North"
  | "NorthEast"
  | "NorthWest"
  | "South"
  | "SouthEast"
  | "SouthWest"
  | "West";

export function ResizeGrips() {
  const maximized = useIsMaximized();

  if (!IS_LINUX) return null;
  if (maximized) return null;

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
