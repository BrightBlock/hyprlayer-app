import { useEffect, useRef, useState } from "react";
import { openPath } from "@tauri-apps/plugin-opener";

import { DocIcon, PencilIcon } from "@/components/icons";
import { PATCH_LINE_CLASSES } from "@/features/workstreams/diff";
import { baseName, type EditDiff } from "./toolDisplay";

const POPOVER_MAX_H = 320;
const POPOVER_W = 520;
const HIDE_DELAY_MS = 120;

type Anchor = { left: number; top?: number; bottom?: number; width: number };

function DiffPopover({
  anchor,
  diff,
  path,
  onMouseEnter,
  onMouseLeave,
}: {
  anchor: Anchor;
  diff: EditDiff;
  path: string;
  onMouseEnter: () => void;
  onMouseLeave: () => void;
}) {
  return (
    <div
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      className="fixed z-50 overflow-hidden rounded-lg border border-edge bg-panel shadow-2xl"
      style={{ left: anchor.left, top: anchor.top, bottom: anchor.bottom, width: anchor.width }}
    >
      <div className="flex items-center gap-2 border-b border-edge px-2.5 py-1.5">
        <span className="min-w-0 flex-1 truncate text-meta text-ink-dim" title={path}>
          {path}
        </span>
        {diff.additions > 0 && (
          <span className="shrink-0 text-meta text-ok">+{diff.additions}</span>
        )}
        {diff.deletions > 0 && (
          <span className="shrink-0 text-meta text-bad">-{diff.deletions}</span>
        )}
      </div>
      <pre
        className="selectable overflow-auto p-2 text-meta"
        style={{ maxHeight: POPOVER_MAX_H - 32 }}
      >
        {diff.lines.map((line, index) => (
          <div key={index} className={`whitespace-pre px-1 ${PATCH_LINE_CLASSES[line.kind]}`}>
            {line.text || " "}
          </div>
        ))}
      </pre>
    </div>
  );
}

export function FileCard({ path, diff }: { path: string; diff?: EditDiff | null }) {
  const ref = useRef<HTMLSpanElement>(null);
  const hideTimer = useRef<number | null>(null);
  const [anchor, setAnchor] = useState<Anchor | null>(null);

  const cancelHide = () => {
    if (hideTimer.current !== null) {
      clearTimeout(hideTimer.current);
      hideTimer.current = null;
    }
  };
  const scheduleHide = () => {
    cancelHide();
    hideTimer.current = window.setTimeout(() => setAnchor(null), HIDE_DELAY_MS);
  };
  useEffect(() => cancelHide, []);

  const show = () => {
    if (!diff) return;
    cancelHide();
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const width = Math.min(POPOVER_W, window.innerWidth - 16);
    const left = Math.max(8, Math.min(rect.left, window.innerWidth - width - 8));
    const spaceBelow = window.innerHeight - rect.bottom;
    if (spaceBelow >= POPOVER_MAX_H || spaceBelow >= rect.top) {
      setAnchor({ left, top: rect.bottom + 6, width });
    } else {
      setAnchor({ left, bottom: window.innerHeight - rect.top + 6, width });
    }
  };

  return (
    <span
      ref={ref}
      onMouseEnter={show}
      onMouseLeave={diff ? scheduleHide : undefined}
      className="inline-flex max-w-56 items-center gap-1 rounded-md border border-accent/40 bg-accent/10 p-0.5 pr-1 text-ui text-ink"
      title={path}
    >
      <span className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded bg-accent/15 text-accent">
        {diff ? <PencilIcon size={12} /> : <DocIcon size={12} />}
      </span>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          void openPath(path).catch(() => {});
        }}
        className="min-w-0 truncate px-0.5 text-left font-medium hover:text-accent"
        title={`Open ${baseName(path)}`}
      >
        {baseName(path)}
      </button>
      {anchor && diff && (
        <DiffPopover
          anchor={anchor}
          diff={diff}
          path={path}
          onMouseEnter={cancelHide}
          onMouseLeave={scheduleHide}
        />
      )}
    </span>
  );
}
