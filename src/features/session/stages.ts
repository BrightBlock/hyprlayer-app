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

/** Stages a user can pick as an agent mode. Ship is excluded on purpose —
 *  shipping is driven by the header "Ship" action (commit + push + PR), not
 *  chosen as a chat mode. Ship stays in [`STAGES`] so its icon/colour/label
 *  still resolve for the dedicated ship tab. */
export const MODE_STAGES = STAGES.filter((s) => s.key !== "ship");

const STAGE_KEYS = new Set<string>(STAGES.map((s) => s.key));

/** The instruction the Ship action runs. Kept in lockstep with the backend
 *  `compose_prompt` (crates/hyprlayer-core/src/pipeline.rs) so the optimistic
 *  transcript echo matches what claude actually receives. */
export const SHIP_PROMPT =
  "Commit all changes in this worktree, push the branch to its remote, then open a " +
  "pull request against the default branch with a clear title and a summary of what changed.";

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
 * The skill slash-command a stage invokes, or `null` for stages that send a
 * plain prompt (general chat; ship sends a prose directive — see [`SHIP_PROMPT`]).
 * Mirror of the backend `compose_prompt` (crates/hyprlayer-core/src/pipeline.rs);
 * the backend is the source of truth for what's sent — this drives the composer
 * hint and the optimistic transcript echo.
 */
export function stageCommand(stage: string): string | null {
  switch (asStageKey(stage)) {
    case "research":
      return "/research_codebase";
    case "plan":
      return "/create_plan";
    case "implement":
      return "/implement_plan";
    case "validate":
      return "/validate_plan";
    default:
      return null; // general, ship
  }
}

function appendDocs(prompt: string, docs: string[]): string {
  if (docs.length === 0) return prompt;
  return prompt + "\n\nRelevant documents:" + docs.map((d) => `\n- ${d}`).join("");
}

/**
 * Compose the first-turn prompt for a stage exactly as the backend does, so the
 * optimistic user bubble matches what claude actually receives. Mirror of
 * `compose_prompt` in crates/hyprlayer-core/src/pipeline.rs — keep the two in
 * sync (covered by stages.test.ts + the Rust unit tests).
 */
export function composeStagePrompt(stage: string, input: string, docs: string[]): string {
  const trimmed = input.trim();
  const key = asStageKey(stage);
  // Plain chat (general): verbatim, no doc list — matches the backend.
  if (key === "general") return trimmed;
  if (key === "ship") {
    return appendDocs(trimmed ? `${SHIP_PROMPT}\n\n${trimmed}` : SHIP_PROMPT, docs);
  }
  const command = stageCommand(key)!; // research/plan/implement/validate
  return appendDocs(trimmed ? `${command} ${trimmed}` : command, docs);
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
