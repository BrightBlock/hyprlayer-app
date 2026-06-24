/**
 * The pipeline stages and their per-stage visual treatment, shared by the
 * project board composer (ProjectPage) and the live chat (SessionChat) so the
 * stage colours never drift between the two surfaces.
 *
 * The backend `stage` is a plain string ("general | research | … | ship");
 * anything it doesn't recognise renders as general chat.
 */

import type { WorkstreamView } from "../../lib/bindings";
import type { DocKind } from "./toolDisplay";

export const STAGES = [
  { key: "general", label: "General" },
  { key: "research", label: "Research" },
  { key: "plan", label: "Plan" },
  { key: "implement", label: "Implement" },
  { key: "validate", label: "Validate" },
  { key: "commit", label: "Commit" },
  { key: "ship", label: "Ship" },
  { key: "revise", label: "Revise" },
] as const;

export type StageKey = (typeof STAGES)[number]["key"];

/** Stages a user can pick as an agent mode. Commit, Ship and Revise are excluded
 *  on purpose — all are driven by header/PR actions (the "Commit" and "Ship"
 *  buttons and the PR bar's feedback actions), not chosen as a chat mode. They
 *  stay in [`STAGES`] so their icon/colour/label still resolve for their tabs. */
export const MODE_STAGES = STAGES.filter(
  (s) => s.key !== "commit" && s.key !== "ship" && s.key !== "revise",
);

const STAGE_KEYS = new Set<string>(STAGES.map((s) => s.key));

/** The instruction the Ship action runs. Kept in lockstep with the backend
 *  `compose_prompt` (crates/hyprlayer-core/src/pipeline.rs) so the optimistic
 *  transcript echo matches what claude actually receives. */
export const SHIP_PROMPT =
  "First commit all changes in this worktree, then push the branch to its remote. Only " +
  "after the commit and push succeed, open a pull request against the default branch with a " +
  "clear title and run the /describe_pr skill to write the PR description from the " +
  "repository's template and update the PR.";

/** The instruction the PR-feedback actions (Fix CI / Address comments) run when
 *  they start a fresh session. Kept byte-identical to the backend `REVISE_PROMPT`
 *  (crates/hyprlayer-core/src/pipeline.rs); the specific feedback rides in the
 *  per-action input appended below it. */
export const REVISE_PROMPT =
  "A pull request already exists for this branch. Address the outstanding feedback, then " +
  "commit and push the fixes to the same branch. Do not open a new pull request.";

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

/** The next selectable agent stage after `stage`, wrapping around — the order
 *  follows [`MODE_STAGES`] (general → research → plan → implement → validate →
 *  general). Drives the "cycle agent stage" keybinding. A stage outside
 *  [`MODE_STAGES`] (commit/ship/revise, or anything unknown) starts the cycle at
 *  the first mode. */
export function nextModeStage(stage: string): StageKey {
  // Exact match (callers pass a normalized StageKey): a stage outside the cycle
  // lands at index -1, so (-1 + 1) % n = 0 — the first mode.
  const i = MODE_STAGES.findIndex((s) => s.key === stage);
  return MODE_STAGES[(i + 1) % MODE_STAGES.length].key;
}

/** Which doc kind a pane in this stage should suggest, or null for none. */
export function suggestedDocKind(stage: string): DocKind | null {
  switch (asStageKey(stage)) {
    case "plan":
      return "research"; // planning builds on research
    case "implement":
      return "plan"; // implementing builds on a plan
    case "validate":
      return "plan"; // validating checks against a plan
    default:
      return null; // general, research → no suggestion
  }
}

/**
 * The skill slash-command a stage invokes, or `null` for stages that send a
 * plain prompt (general chat; ship and revise send prose directives — see
 * [`SHIP_PROMPT`], [`REVISE_PROMPT`]). Mirror of the backend `compose_prompt`
 * (crates/hyprlayer-core/src/pipeline.rs); the backend is the source of truth
 * for what's sent — this drives the composer hint and the optimistic transcript
 * echo.
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
    case "commit":
      return "/commit";
    default:
      return null; // general, ship, revise
  }
}

/**
 * The skill-prefixed prompt the optimistic user bubble echoes — the skill
 * command + the user's input, matching the head of the backend `compose_prompt`
 * (crates/hyprlayer-core/src/pipeline.rs). Attached docs are NOT echoed here:
 * they render as pin cards and the backend owns the `Relevant documents:` tail.
 */
export function composeStagePrompt(stage: string, input: string): string {
  const trimmed = input.trim();
  const key = asStageKey(stage);
  if (key === "general") return trimmed; // plain chat: no skill prefix
  if (key === "ship") return trimmed ? `${SHIP_PROMPT}\n\n${trimmed}` : SHIP_PROMPT;
  if (key === "revise") return trimmed ? `${REVISE_PROMPT}\n\n${trimmed}` : REVISE_PROMPT;
  const command = stageCommand(key)!; // research/plan/implement/validate/commit
  return trimmed ? `${command} ${trimmed}` : command;
}

/**
 * Tailwind class fragments per stage: the active mode button, the inline
 * badge, the composer textarea border, and the standalone icon accent (the
 * text-hue alone, for a colored icon on a neutral chip — see the board tiles).
 * Each stage binds one themed accent token (research/plan/implement/validate/
 * revise, plus the shared commit/ship tokens) rather than a hardcoded Tailwind
 * shade, so every hue recolors per theme and deepens on the light surface
 * instead of washing out. The tokens live in src/index.css; commit/ship reuse
 * the header action-button tokens so a stage badge matches its header action.
 */
export const MODE_STYLES: Record<
  StageKey,
  {
    activeButton: string;
    badge: string;
    composer: string;
    text: string;
  }
> = {
  general: {
    activeButton: "border-edge bg-edge/60 text-ink",
    badge: "bg-edge/60 text-ink-dim",
    composer: "border-edge",
    text: "text-ink-dim",
  },
  research: {
    activeButton: "border-research/40 bg-research/10 text-research",
    badge: "bg-research/10 text-research",
    composer: "border-research/40",
    text: "text-research",
  },
  plan: {
    activeButton: "border-plan/40 bg-plan/10 text-plan",
    badge: "bg-plan/10 text-plan",
    composer: "border-plan/40",
    text: "text-plan",
  },
  implement: {
    activeButton: "border-implement/40 bg-implement/10 text-implement",
    badge: "bg-implement/10 text-implement",
    composer: "border-implement/40",
    text: "text-implement",
  },
  validate: {
    activeButton: "border-validate/40 bg-validate/10 text-validate",
    badge: "bg-validate/10 text-validate",
    composer: "border-validate/40",
    text: "text-validate",
  },
  commit: {
    activeButton: "border-commit/40 bg-commit/10 text-commit",
    badge: "bg-commit/10 text-commit",
    composer: "border-commit/40",
    text: "text-commit",
  },
  ship: {
    activeButton: "border-ship/40 bg-ship/10 text-ship",
    badge: "bg-ship/10 text-ship",
    composer: "border-ship/40",
    text: "text-ship",
  },
  revise: {
    activeButton: "border-revise/40 bg-revise/10 text-revise",
    badge: "bg-revise/10 text-revise",
    composer: "border-revise/40",
    text: "text-revise",
  },
};
