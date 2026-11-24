import { useId, type KeyboardEvent } from "react";

import { Icon, type IconName } from "@/components/Icon";
import { Button } from "@/components/ui";
import { STATUS_BAR_HEIGHT } from "@/lib/layout";
import { useCardContent } from "./actions";
import { ALSO_NEW, CARD_LINES, CARD_TITLES } from "./steps";
import { dismissWhatsNewCard, startTour, useTourStore } from "./tourStore";

const ROW_ICON: Record<string, IconName> = {
  "wn.panel": "panels",
  "wn.queue": "queued",
  "wn.remote": "leadRemote",
  "wn.resume": "history",
  "wn.server": "server",
};

/**
 * Bottom right, above the status bar. It never takes focus: its arrival is announced once, politely, and
 * Esc dismisses it only while focus is inside it.
 */
export function WhatsNewCard() {
  const open = useTourStore((s) => s.cardOpen);
  const { steps, since, replay } = useCardContent();
  const titleId = useId();
  if (!open || !steps.length) return null;

  const dismiss = () => dismissWhatsNewCard(steps);
  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    dismiss();
  };
  const also = ALSO_NEW[steps[0].since];

  return (
    <section
      role="region"
      aria-labelledby={titleId}
      onKeyDown={onKeyDown}
      className="tour-in fixed right-4 z-40 w-[392px] rounded-card border border-edge bg-panel shadow-pop"
      style={{ bottom: STATUS_BAR_HEIGHT + 12 }}
    >
      <p className="sr-only" role="status" aria-live="polite">
        What’s new is open in the bottom right.
      </p>
      <div className="flex items-center gap-2 pt-3 pr-2 pl-4">
        <span className="text-label text-ink-dim uppercase">
          {replay ? `What’s new in ${since}` : `What’s new since ${since}`}
        </span>
        <span className="flex-1" />
        <button
          type="button"
          aria-label="Dismiss what’s new"
          title="Dismiss (Esc)"
          onClick={dismiss}
          className="flex h-6 w-6 items-center justify-center rounded-control text-ink-dim hover:bg-edge/50 hover:text-ink focus-visible:bg-edge/50 focus-visible:text-ink focus:outline-none"
        >
          <Icon name="close" size="xs" />
        </button>
      </div>
      <h2 id={titleId} className="px-4 pt-1.5 pb-1 text-title text-ink">
        The Lead got a few new tricks
      </h2>
      <ul className="pr-3 pl-4">
        {steps.map((step) => (
          <li key={step.id} className="flex items-start gap-2.5 border-t border-edge-2 py-2.5">
            <Icon name={ROW_ICON[step.id] ?? "info"} size="sm" className="mt-px text-ink-dim" />
            <div className="min-w-0 flex-1">
              <p className="text-ui font-semibold text-ink">{CARD_TITLES[step.id] ?? step.title}</p>
              <p className="mt-0.5 text-meta text-ink-dim">{CARD_LINES[step.id] ?? ""}</p>
            </div>
            <button
              type="button"
              aria-label={`Show me: ${CARD_TITLES[step.id] ?? step.title}`}
              onClick={() => startTour("whatsNew", steps, step.id)}
              className="h-6 shrink-0 rounded-control px-2 text-ui text-ink-2 hover:bg-edge/50 hover:text-ink focus-visible:bg-edge/50 focus:outline-none"
            >
              Show me
            </button>
          </li>
        ))}
      </ul>
      {also && <p className="border-t border-edge-2 px-4 pt-2 text-meta text-ink-dim">Also: {also}</p>}
      <div className="flex items-center gap-1.5 pt-3 pr-3 pb-3 pl-2">
        <button
          type="button"
          onClick={dismiss}
          className="h-7 rounded-control px-2 text-ui text-ink-dim hover:bg-edge/50 hover:text-ink focus-visible:bg-edge/50 focus:outline-none"
        >
          Later
        </button>
        <span className="flex-1" />
        <Button size="sm" variant="primary" onClick={() => startTour("whatsNew", steps)}>
          <Icon name="run" size="xs" />
          Take the {steps.length}-step tour
        </Button>
      </div>
    </section>
  );
}
