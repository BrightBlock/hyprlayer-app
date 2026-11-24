import {
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";

import { Icon } from "@/components/Icon";
import { cx, IconButton } from "@/components/ui";
import type { LeadQueueItem } from "@/lib/bindings";
import { errorMessage } from "@/lib/ipc";
import { useRemoveLeadQueueItem, useReorderLeadQueue } from "@/lib/queries";
import { useToast } from "@/lib/toast";

const DRAG_THRESHOLD_PX = 4;

export function movedTo<T>(list: readonly T[], from: number, to: number): T[] {
  const next = [...list];
  next.splice(to, 0, ...next.splice(from, 1));
  return next;
}

export function keyTarget(key: string, index: number, count: number): number | null {
  switch (key) {
    case "ArrowUp":
      return index - 1;
    case "ArrowDown":
      return index + 1;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}

type HandleEvents = {
  onPointerDown: (e: ReactPointerEvent<HTMLButtonElement>) => void;
  onPointerMove: (e: ReactPointerEvent<HTMLButtonElement>) => void;
  onPointerUp: (e: ReactPointerEvent<HTMLButtonElement>) => void;
  onKeyDown: (e: KeyboardEvent<HTMLButtonElement>) => void;
};

// Rows reorder on pointer events, not HTML5 drag-and-drop: Tauri's dragDropEnabled swallows dragstart (see HostList).
export function LeadQueueList({ projectPath, items }: { projectPath: string; items: LeadQueueItem[] }) {
  const reorder = useReorderLeadQueue();
  const listRef = useRef<HTMLUListElement>(null);
  const drag = useRef<{ from: number; startY: number; moved: boolean } | null>(null);
  const refocus = useRef<string | null>(null);
  const [dragging, setDragging] = useState<number | null>(null);
  const [over, setOver] = useState<number | null>(null);
  const [announcement, setAnnouncement] = useState("");
  const hintId = useId();

  // A moved row remounts at its new index; keyboard reorder keeps focus on its handle.
  useEffect(() => {
    const id = refocus.current;
    if (!id) return;
    refocus.current = null;
    const handles = listRef.current?.querySelectorAll<HTMLElement>("[data-handle]") ?? [];
    Array.from(handles)
      .find((h) => h.dataset.handle === id)
      ?.focus();
  });

  const move = (from: number, to: number) => {
    if (from === to || to < 0 || to >= items.length) return;
    const item = items[from];
    refocus.current = item.id;
    setAnnouncement(`${item.title} moved to position ${to + 1} of ${items.length}`);
    reorder.mutate(
      { projectPath, ids: movedTo(items, from, to).map((i) => i.id) },
      { onError: (e) => useToast.getState().show("Couldn't reorder the queue", errorMessage(e), { tone: "bad" }) },
    );
  };

  const rowAt = (clientY: number): number | null => {
    const rows = Array.from(listRef.current?.querySelectorAll<HTMLElement>("li[data-row]") ?? []);
    if (!rows.length) return null;
    const hit = rows.findIndex((li) => {
      const r = li.getBoundingClientRect();
      return clientY >= r.top && clientY <= r.bottom;
    });
    if (hit !== -1) return hit;
    return clientY < rows[0].getBoundingClientRect().top ? 0 : rows.length - 1;
  };

  const handle = (index: number): HandleEvents => ({
    onPointerDown: (e) => {
      if (e.button !== 0) return;
      drag.current = { from: index, startY: e.clientY, moved: false };
    },
    onPointerMove: (e) => {
      const state = drag.current;
      if (!state) return;
      if (!state.moved) {
        if (Math.abs(e.clientY - state.startY) < DRAG_THRESHOLD_PX) return;
        state.moved = true;
        e.currentTarget.setPointerCapture(e.pointerId);
        setDragging(state.from);
      }
      setOver(rowAt(e.clientY));
    },
    onPointerUp: (e) => {
      const state = drag.current;
      drag.current = null;
      if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
      const to = rowAt(e.clientY);
      if (state?.moved && to !== null) move(state.from, to);
      setDragging(null);
      setOver(null);
    },
    onKeyDown: (e) => {
      const to = keyTarget(e.key, index, items.length);
      if (to === null) return;
      e.preventDefault();
      move(index, to);
    },
  });

  return (
    <>
      <ul ref={listRef}>
        {items.map((item, index) => (
          <LeadQueueRow
            key={item.id}
            item={item}
            position={index}
            count={items.length}
            dragging={dragging === index}
            dropTarget={dragging !== null && over === index && over !== dragging}
            handle={handle(index)}
            hintId={hintId}
          />
        ))}
      </ul>
      <p id={hintId} className="sr-only">
        Press the up or down arrow to move it.
      </p>
      <p role="status" aria-live="polite" className="sr-only">
        {announcement}
      </p>
    </>
  );
}

function LeadQueueRow({
  item,
  position,
  count,
  dragging,
  dropTarget,
  handle,
  hintId,
}: {
  item: LeadQueueItem;
  position: number;
  count: number;
  dragging: boolean;
  dropTarget: boolean;
  handle: HandleEvents;
  hintId: string;
}) {
  const remove = useRemoveLeadQueueItem();
  const drop = () =>
    remove.mutate(
      { projectPath: item.projectPath, id: item.id },
      {
        onError: (e) =>
          useToast.getState().show(`Couldn't remove ${item.title}`, errorMessage(e), { tone: "bad" }),
      },
    );
  return (
    <li
      data-row
      className={cx(
        "group flex items-start gap-1 border-t border-edge-2 py-2 pr-1.5 pl-1",
        dragging && "opacity-50",
        dropTarget && "bg-edge/50",
      )}
    >
      <button
        type="button"
        data-handle={item.id}
        aria-label={`Reorder ${item.title}, position ${position + 1} of ${count}`}
        aria-describedby={hintId}
        title="Drag, or press ↑ and ↓, to reorder"
        {...handle}
        className="flex h-6 w-5 shrink-0 cursor-grab touch-none items-center justify-center rounded-control text-ink-4 hover:bg-edge/50 hover:text-ink focus-visible:bg-edge/50 focus-visible:text-ink active:cursor-grabbing"
      >
        <Icon name="dragHandle" size="xs" />
      </button>
      <div className="min-w-0 flex-1">
        <p className="truncate text-body text-ink">{item.title}</p>
        <p className="line-clamp-2 text-meta text-ink-dim">{item.summary}</p>
      </div>
      <IconButton
        aria-label={`Remove ${item.title} from the queue`}
        title="Remove from the queue"
        disabled={remove.isPending}
        onClick={drop}
        className="h-6 w-6"
      >
        <Icon name="close" size="xs" />
      </IconButton>
    </li>
  );
}
