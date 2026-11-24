import { useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";

import { Icon } from "@/components/Icon";
import { Button, cx, Kbd } from "@/components/ui";
import { formatChord } from "@/lib/keybindings/chord";
import { effectiveChord } from "@/lib/keybindings/resolve";
import { IS_MACOS } from "@/lib/platform";
import { placeCoach, type Placement, type Rect } from "./place";
import type { Side, TourPath, TourStep } from "./steps";

const WIDTH = 344;
const INTRO_WIDTH = 480;
const PATH_LABEL: Record<TourPath, string> = { firstRun: "First run", whatsNew: "What’s new" };

/** Renders `{chord:<command id>}` as the user's binding for that command. */
export function StepBody({ text, overrides }: { text: string; overrides: Record<string, string | null> }) {
  const parts = text.split(/(\{chord:[\w.]+\})/g);
  return (
    <>
      {parts.map((part, i) => {
        const m = part.match(/^\{chord:([\w.]+)\}$/);
        if (!m) return part;
        const chord = effectiveChord(overrides, m[1], IS_MACOS);
        return chord ? <Kbd key={i} chord={chord} className="mx-0.5 align-baseline" /> : null;
      })}
    </>
  );
}

export function chordText(overrides: Record<string, string | null>, id: string): string {
  const chord = effectiveChord(overrides, id, IS_MACOS);
  return chord ? formatChord(chord, IS_MACOS) : "";
}

export type TryState = { kind: "waiting" } | { kind: "done" };

export type CoachMarkProps = {
  step: TourStep;
  path: TourPath;
  index: number;
  total: number;
  /** Null: shown unanchored in the main area, with `notice` saying why. */
  anchor: Rect | null;
  /** An intro card: wider, centred, over a scrim. */
  intro?: boolean;
  /** A second control the step names, outlined with a label. */
  also?: { rect: Rect; label: string };
  notice?: ReactNode;
  fix?: ReactNode;
  tryState?: TryState;
  canAdvance: boolean;
  overrides: Record<string, string | null>;
  onNext: () => void;
  onBack: () => void;
  onSkipStep: () => void;
  onPause: () => void;
  onEndTour: () => void;
  onUsePrompt?: () => void;
  onFocusAnchor?: () => void;
};

const CARET_EDGE: Record<Side, string> = {
  // The caret sits on the edge that faces the anchor, which is opposite the side the card took.
  top: "-bottom-[5px] border-b border-r",
  bottom: "-top-[5px] border-l border-t",
  left: "-right-[5px] border-r border-t",
  right: "-left-[5px] border-l border-b",
};

export function CoachMark(props: CoachMarkProps) {
  const { step, path, index, total, anchor, intro, also, tryState, canAdvance, overrides } = props;
  const width = intro ? INTRO_WIDTH : WIDTH;
  const panel = useRef<HTMLDivElement>(null);
  const title = useRef<HTMLHeadingElement>(null);
  const [place, setPlace] = useState<Placement | null>(null);
  const titleId = `tour-${step.id}-title`;
  const bodyId = `tour-${step.id}-body`;

  // Focus moves to the title once per step; nothing is trapped, so Tab carries on into the app.
  useLayoutEffect(() => {
    title.current?.focus({ preventScroll: true });
  }, [step.id]);

  useLayoutEffect(() => {
    const el = panel.current;
    if (!el || !anchor) {
      setPlace(null);
      return;
    }
    const view = { width: window.innerWidth, height: window.innerHeight };
    setPlace(placeCoach(anchor, { width: el.offsetWidth, height: el.offsetHeight }, step.side, view));
  }, [anchor, step.side, tryState?.kind, props.notice]);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "F6" && props.onFocusAnchor) {
      e.preventDefault();
      // The app-wide F6 handler would send focus straight back here.
      e.stopPropagation();
      props.onFocusAnchor();
    }
  };

  const style = anchor
    ? place
      ? { left: place.left, top: place.top }
      : { left: -10000, top: 0 }
    : // Unanchored: centred, with numbers rather than a transform the entrance keyframes would fight.
      {
        left: Math.max(8, (window.innerWidth - width) / 2),
        top: Math.round(window.innerHeight * (intro ? 0.14 : 0.3)),
      };

  const last = index === total - 1;
  return createPortal(
    <>
      {anchor && <Highlight rect={anchor} />}
      {anchor && also && <AlsoMark rect={also.rect} label={also.label} />}
      {intro && (
        <div
          aria-hidden="true"
          data-tour-scrim=""
          className="tour-in pointer-events-none fixed inset-0 z-[59]"
          style={{ background: "var(--tour-scrim)" }}
        />
      )}
      <div
        ref={panel}
        role="dialog"
        aria-modal="false"
        aria-labelledby={titleId}
        aria-describedby={bodyId}
        data-tour-coach=""
        onKeyDown={onKeyDown}
        className="tour-in fixed z-[60] rounded-card border border-edge bg-panel shadow-pop"
        style={{ width, ...style }}
      >
        {anchor && place && (
          <span
            aria-hidden="true"
            className={cx("absolute -z-10 h-[9px] w-[9px] rotate-45 border-edge bg-panel", CARET_EDGE[place.side])}
            style={place.side === "top" || place.side === "bottom" ? { left: place.caret } : { top: place.caret }}
          />
        )}
        <div className="flex items-center gap-2.5 pt-3 pr-2 pl-4">
          <span className="text-label whitespace-nowrap text-ink-dim uppercase">
            {PATH_LABEL[path]} · {index + 1} of {total}
          </span>
          <span aria-hidden="true" className="flex gap-[3px]">
            {Array.from({ length: total }, (_, i) => (
              <span
                key={i}
                className={cx(
                  "h-[3px] rounded-[1px]",
                  total > 6 ? "w-[11px]" : "w-[18px]",
                  i < index ? "bg-ok" : i === index ? "bg-accent" : "bg-edge",
                )}
              />
            ))}
          </span>
          <span className="flex-1" />
          <button
            type="button"
            aria-label="End the tour for now"
            title="Pause the tour (Esc)"
            onClick={props.onPause}
            className="flex h-6 w-6 items-center justify-center rounded-control text-ink-dim hover:bg-edge/50 hover:text-ink focus-visible:bg-edge/50 focus-visible:text-ink focus:outline-none"
          >
            <Icon name="close" size="xs" />
          </button>
        </div>
        <div className={intro ? "px-5 pt-3" : "px-4 pt-2"}>
          <h2
            ref={title}
            id={titleId}
            tabIndex={-1}
            className={cx(intro ? "text-h3" : "text-title", "text-ink focus:outline-none")}
          >
            {step.title}
          </h2>
          <div id={bodyId} className={cx("mt-1.5 flex flex-col gap-2 text-ink-2", intro ? "text-body-lg" : "text-body")}>
            {step.body
              .split("\n\n")
              .filter(Boolean)
              .map((para, i) => (
                <p key={i}>
                  <StepBody text={para} overrides={overrides} />
                </p>
              ))}
            {step.points && <Points points={step.points} />}
            {step.note && <p className="text-meta text-ink-dim">{step.note}</p>}
          </div>
          {props.notice && (
            <div className="mt-3 rounded-card border border-edge bg-surface px-3 py-2.5 text-meta text-ink-2">
              {props.notice}
            </div>
          )}
          {props.fix && <div className="mt-3 flex flex-wrap gap-1.5">{props.fix}</div>}
          {step.tryIt && tryState && (
            <TryIt step={step} state={tryState} onUsePrompt={props.onUsePrompt} onSkip={props.onSkipStep} />
          )}
        </div>
        <div className="flex items-center gap-1.5 pt-3.5 pr-3 pb-3 pl-2">
          <button
            type="button"
            onClick={props.onEndTour}
            className="h-7 rounded-control px-2 text-ui text-ink-dim hover:bg-edge/50 hover:text-ink focus-visible:bg-edge/50 focus:outline-none"
          >
            Skip tour
          </button>
          <span className="flex-1" />
          {index > 0 && (
            <button
              type="button"
              onClick={props.onBack}
              className="h-7 rounded-control px-2 text-ui text-ink-dim hover:bg-edge/50 hover:text-ink focus-visible:bg-edge/50 focus:outline-none"
            >
              Back
            </button>
          )}
          {!canAdvance && !step.tryIt && (
            <Button size="sm" onClick={props.onSkipStep}>
              Skip this step
            </Button>
          )}
          <Button size="sm" variant="primary" disabled={!canAdvance} onClick={props.onNext}>
            {last ? (path === "firstRun" ? "Finish" : "Done") : "Next"}
          </Button>
        </div>
      </div>
    </>,
    document.body,
  );
}

function TryIt({
  step,
  state,
  onUsePrompt,
  onSkip,
}: {
  step: TourStep;
  state: TryState;
  onUsePrompt?: () => void;
  onSkip: () => void;
}) {
  const t = step.tryIt!;
  return (
    <div className="mt-3 rounded-card border border-edge bg-surface">
      <div className="px-3 pt-2.5">
        <span className="inline-flex h-4 items-center rounded-control border border-edge px-1.5 text-tag text-ink">
          TRY IT
        </span>
      </div>
      {t.prompt && <p className="px-3 pt-2 text-body text-ink">{t.prompt}</p>}
      {t.prompt && onUsePrompt && state.kind === "waiting" && (
        <div className="px-3 pt-2.5">
          <Button size="sm" onClick={onUsePrompt}>
            {t.promptLabel ?? "Use this"}
          </Button>
        </div>
      )}
      <div role="status" aria-live="polite" className="flex items-center gap-2 px-3 py-2.5">
        {state.kind === "done" ? (
          <>
            <Icon name="check" size="xs" className="text-ok" />
            <span className="text-meta text-ink-2">{t.done}</span>
          </>
        ) : (
          <>
            <span aria-hidden="true" className="h-[11px] w-[11px] shrink-0 rounded-full border border-ink-4" />
            <span className="text-meta text-ink-dim">{t.waiting}</span>
            <span className="flex-1" />
            <button
              type="button"
              onClick={onSkip}
              className="text-meta text-ink-dim underline hover:text-ink focus-visible:text-ink focus:outline-none"
            >
              Skip this step
            </button>
          </>
        )}
      </div>
    </div>
  );
}

function Points({ points }: { points: NonNullable<TourStep["points"]> }) {
  return (
    <ol className="mt-1 rounded-card border border-edge bg-surface">
      {points.map((p, i) => (
        <li key={p.title} className="grid grid-cols-[22px_1fr] gap-2.5 border-t border-edge-2 px-3.5 py-2.5 first:border-t-0">
          <span
            aria-hidden="true"
            className="flex h-[22px] w-[22px] items-center justify-center rounded-full border border-edge font-mono text-meta text-ink"
          >
            {i + 1}
          </span>
          <span>
            <span className="block text-ui font-semibold text-ink">{p.title}</span>
            <span className="block text-body text-ink-2">{p.text}</span>
          </span>
        </li>
      ))}
    </ol>
  );
}

/** The step's second control: a dashed outline and a label, above the scrim; takes no click. */
function AlsoMark({ rect, label }: { rect: Rect; label: string }) {
  const above = rect.top > 32;
  return (
    <>
      <div
        aria-hidden="true"
        data-tour-also=""
        className="tour-in pointer-events-none fixed z-[59] rounded-card border border-dashed border-ink-dim"
        style={{ left: rect.left - 2, top: rect.top - 2, width: rect.width + 4, height: rect.height + 4 }}
      />
      <span
        aria-hidden="true"
        className="tour-in pointer-events-none fixed z-[59] rounded-control border border-edge bg-panel px-1.5 py-0.5 text-meta whitespace-nowrap text-ink-2"
        style={{
          right: Math.max(8, window.innerWidth - (rect.left + rect.width)),
          top: above ? rect.top - 28 : rect.top + rect.height + 6,
        }}
      >
        {label}
      </span>
    </>
  );
}

/** The anchor's outline and a scrim drawn as its shadow; neither takes a click. */
function Highlight({ rect }: { rect: Rect }) {
  return (
    <div
      aria-hidden="true"
      data-tour-highlight=""
      className="tour-in pointer-events-none fixed z-[59] rounded-card border border-accent"
      style={{
        left: rect.left - 2,
        top: rect.top - 2,
        width: rect.width + 4,
        height: rect.height + 4,
        boxShadow: "0 0 0 100vmax var(--tour-scrim)",
      }}
    />
  );
}
