import { useId, useRef, useState } from "react";

import { Icon } from "@/components/Icon";
import { BAR_POPOVER_ROW, cx, Popover } from "@/components/ui";
import { endTour, pausedTour, resumeTour, startTour, useTourStore } from "./tourStore";

const PATH_LABEL = { firstRun: "First run", whatsNew: "What’s new" } as const;

/** Shown in the status bar only while a tour is paused: it holds the place until the tour ends. */
export function TourResumeButton() {
  const key = useTourStore((s) => {
    const t = pausedTour(s);
    return t ? `${t.path}:${t.index}` : null;
  });
  const active = useTourStore((s) => s.firstRun.status === "active" || s.whatsNew.status === "active");
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const headingId = useId();
  const tour = key && !active ? pausedTour(useTourStore.getState()) : null;
  if (!tour) return null;

  const position = `${tour.index + 1}/${tour.steps.length}`;
  const close = () => setOpen(false);
  return (
    <>
      <button
        ref={anchor}
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`Resume the tour: ${PATH_LABEL[tour.path]}, step ${tour.index + 1} of ${tour.steps.length}`}
        onClick={() => setOpen((o) => !o)}
        className={cx(
          "inline-flex h-6 shrink-0 items-center gap-1.5 rounded-control bg-panel-2 px-2 text-meta text-ink",
          "hover:bg-edge/60 focus-visible:bg-edge/60 focus:outline-none",
        )}
      >
        <Icon name="history" size="xs" />
        <span className="whitespace-nowrap">Resume tour</span>
        <span className="text-ink-dim tabular-nums">{position}</span>
      </button>
      <Popover
        open={open}
        onClose={close}
        anchorRef={anchor}
        side="top"
        caret
        trapFocus
        aria-labelledby={headingId}
        className="w-[300px] p-1.5"
      >
        <p id={headingId} className="px-3 pt-1.5 pb-1 text-label text-ink-dim uppercase">
          {PATH_LABEL[tour.path]} · {tour.index + 1} of {tour.steps.length}
        </p>
        <p className="px-3 pb-2 text-ui font-semibold text-ink">{tour.step.title}</p>
        <button
          type="button"
          onClick={() => {
            close();
            resumeTour();
          }}
          className={cx(BAR_POPOVER_ROW, "w-full bg-panel-2 text-left font-semibold text-ink")}
        >
          <Icon name="run" size="xs" />
          <span className="flex-1">Resume</span>
        </button>
        <button
          type="button"
          onClick={() => {
            close();
            startTour(tour.path, tour.steps);
          }}
          className={cx(BAR_POPOVER_ROW, "w-full text-left")}
        >
          <span className="flex-1">Start over</span>
        </button>
        <button
          type="button"
          onClick={() => {
            close();
            endTour(tour.path);
          }}
          className={cx(BAR_POPOVER_ROW, "w-full text-left")}
        >
          <span className="flex-1">End the tour</span>
        </button>
      </Popover>
    </>
  );
}
