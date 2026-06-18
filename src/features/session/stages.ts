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

/** The instruction the Commit action runs — checkpoint the worktree locally,
 *  without pushing or opening a PR (the split-out counterpart to SHIP_PROMPT).
 *  Kept byte-identical to the backend `COMMIT_PROMPT`
 *  (crates/hyprlayer-core/src/pipeline.rs). */
export const COMMIT_PROMPT =
  "Commit all changes in this worktree: stage everything, then commit with a clear, " +
  "descriptive message. Do not push the branch or open a pull request.";

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
      return null; // general, commit, ship, revise
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
  if (key === "commit") return trimmed ? `${COMMIT_PROMPT}\n\n${trimmed}` : COMMIT_PROMPT;
  if (key === "ship") return trimmed ? `${SHIP_PROMPT}\n\n${trimmed}` : SHIP_PROMPT;
  if (key === "revise") return trimmed ? `${REVISE_PROMPT}\n\n${trimmed}` : REVISE_PROMPT;
  const command = stageCommand(key)!; // research/plan/implement/validate
  return trimmed ? `${command} ${trimmed}` : command;
}

/**
 * Tailwind class fragments per stage: the active mode button, the inline
 * badge, and the composer textarea border. Each stage owns one accent hue
 * (research=sky, plan=amber, implement=emerald, validate=violet, commit=teal,
 * ship=emerald to match the header Ship action).
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
  commit: {
    activeButton: "border-teal-400/40 bg-teal-400/10 text-teal-300",
    badge: "bg-teal-400/10 text-teal-300",
    composer: "border-teal-400/40",
  },
  ship: {
    activeButton: "border-emerald-400/40 bg-emerald-400/10 text-emerald-300",
    badge: "bg-emerald-400/10 text-emerald-300",
    composer: "border-emerald-400/40",
  },
  revise: {
    activeButton: "border-purple-400/40 bg-purple-400/10 text-purple-300",
    badge: "bg-purple-400/10 text-purple-300",
    composer: "border-purple-400/40",
  },
};
