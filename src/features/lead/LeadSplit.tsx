import type { ReactNode } from "react";

import { PaneSeparator } from "@/components/ui";
import type { WorkstreamView } from "@/lib/bindings";
import { useCommand } from "@/lib/keybindings/registry";
import { clamp, maxBeside } from "@/lib/resize";
import {
  LEAD_DOCK_KEEP,
  LEAD_DOCK_MAX,
  LEAD_DOCK_MIN,
  useUiStore,
} from "@/lib/uiStore";
import { useDraggedSize } from "@/lib/useDraggedSize";
import { useElementWidth } from "@/lib/useElementWidth";
import { LeadPanel } from "./LeadPanel";

// The expanded dock's chat and, on its right, the Lead panel behind a splitter.
export function LeadDockSplit({
  projectPath,
  views,
  children,
}: {
  projectPath: string;
  views: WorkstreamView[];
  children: ReactNode;
}) {
  const { ref, width: rowWidth } = useElementWidth<HTMLDivElement>();
  const hidden = useUiStore((s) => s.leadPanelHidden);
  // The app's top bar offers the toggle while this command is registered.
  useCommand("view.toggleLeadPanel", useUiStore((s) => s.toggleLeadPanel));
  const stored = useUiStore((s) => s.leadDockWidth);
  const store = useUiStore((s) => s.setLeadDockWidth);
  const drag = useDraggedSize(stored, store);

  const max = maxBeside(rowWidth, LEAD_DOCK_KEEP, LEAD_DOCK_MIN, LEAD_DOCK_MAX);
  const width = clamp(drag.size, LEAD_DOCK_MIN, max);
  return (
    <div ref={ref} className="flex min-h-0 flex-1 border-t border-edge-2">
      {children}
      {!hidden && (
        <>
          <PaneSeparator
            label="Resize the Lead panel"
            direction={-1}
            size={width}
            min={LEAD_DOCK_MIN}
            max={max}
            onResize={drag.onResize}
            onEnd={drag.onEnd}
          />
          <div className="flex min-h-0 shrink-0 flex-col border-l border-edge" style={{ width }}>
            <LeadPanel projectPath={projectPath} views={views} className="min-h-0 flex-1" />
          </div>
        </>
      )}
    </div>
  );
}
