/**
 * The pipeline stages and their per-stage visual treatment, shared by the
 * project board composer (ProjectPage) and the live chat (SessionChat) so the
 * stage colours never drift between the two surfaces.
 *
 * The backend `stage` is a plain string ("general | research | … | ship");
 * anything it doesn't recognise renders as general chat.
 */

import type { WorkstreamView } from "../../lib/bindings";

export const STAGES = [
  { key: "general", label: "General" },
  { key: "research", label: "Research" },
  { key: "plan", label: "Plan" },
  { key: "implement", label: "Implement" },
  { key: "validate", label: "Validate" },
  { key: "ship", label: "Ship" },
] as const;

export type StageKey = (typeof STAGES)[number]["key"];

const STAGE_KEYS = new Set<string>(STAGES.map((s) => s.key));

/** Backend stage is a plain string; anything unknown renders as general. */
export function stageOf(view: WorkstreamView): StageKey {
  return (STAGE_KEYS.has(view.item.stage) ? view.item.stage : "general") as StageKey;
}

/** Normalise an arbitrary stage string to a known key (default general). */
export function asStageKey(stage: string): StageKey {
  return (STAGE_KEYS.has(stage) ? stage : "general") as StageKey;
}

export function stageLabel(stage: string): string {
  return STAGES.find((s) => s.key === asStageKey(stage))!.label;
}

/**
 * Tailwind class fragments per stage: the active mode button, the inline
 * badge, and the composer textarea border. Each stage owns one accent hue
 * (research=sky, plan=amber, implement=emerald, validate=violet, ship=rose).
 */
export const MODE_STYLES: Record<
  StageKey,
  {
    activeButton: string;
    badge: string;
    composer: string;
  }
> = {
  general: {
    activeButton: "border-edge bg-edge/60 text-ink",
    badge: "bg-edge/60 text-ink-dim",
    composer: "border-edge",
  },
  research: {
    activeButton: "border-sky-400/40 bg-sky-400/10 text-sky-300",
    badge: "bg-sky-400/10 text-sky-300",
    composer: "border-sky-400/40",
  },
  plan: {
    activeButton: "border-amber-400/40 bg-amber-400/10 text-amber-300",
    badge: "bg-amber-400/10 text-amber-300",
    composer: "border-amber-400/40",
  },
  implement: {
    activeButton: "border-emerald-400/40 bg-emerald-400/10 text-emerald-300",
    badge: "bg-emerald-400/10 text-emerald-300",
    composer: "border-emerald-400/40",
  },
  validate: {
    activeButton: "border-violet-400/40 bg-violet-400/10 text-violet-300",
    badge: "bg-violet-400/10 text-violet-300",
    composer: "border-violet-400/40",
  },
  ship: {
    activeButton: "border-rose-400/40 bg-rose-400/10 text-rose-300",
    badge: "bg-rose-400/10 text-rose-300",
    composer: "border-rose-400/40",
  },
};
