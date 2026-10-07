import { useState, type ReactNode } from "react";

import { onListNavKeyDown } from "@/lib/listNav";
import { cx, PaneSeparator } from "./ui";

// Nav, resizable list, detail. Reads no stores: each feature passes its own layout state.
export function PaneFrame({
  nav,
  list,
  detail,
  navWidth,
  navShown,
  overlayNav,
  listWidth,
  listMin,
  listMax,
  onListWidth,
  listLabel,
}: {
  nav: ReactNode;
  list: ReactNode;
  detail: ReactNode;
  navWidth: number;
  navShown: boolean;
  overlayNav: boolean;
  listWidth: number;
  listMin: number;
  listMax: number;
  onListWidth: (width: number) => void;
  listLabel: string;
}) {
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const width = overlayNav ? listWidth : (dragWidth ?? listWidth);

  return (
    <div className="relative flex h-full min-h-0">
      {navShown && (
        <aside
          className={cx(
            "flex shrink-0 flex-col border-r border-edge bg-panel",
            overlayNav && "absolute inset-y-0 left-0 z-30 shadow-lg",
          )}
          style={{ width: navWidth }}
        >
          {nav}
        </aside>
      )}
      <div
        className="flex min-h-0 shrink-0 flex-col border-r border-edge"
        style={{ width }}
        onKeyDown={onListNavKeyDown}
      >
        {list}
      </div>
      {/* The overlay layout is the narrow one, where the list width is fixed. */}
      {!overlayNav && (
        <PaneSeparator
          label={listLabel}
          width={listWidth}
          min={listMin}
          max={listMax}
          onResize={setDragWidth}
          onEnd={() =>
            setDragWidth((live) => {
              if (live !== null) onListWidth(live);
              return null;
            })
          }
        />
      )}
      <section className="min-w-0 flex-1 overflow-y-auto">{detail}</section>
    </div>
  );
}
