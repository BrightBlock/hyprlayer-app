/**
 * Per-stage labels and visual treatment, shared by the project board composer
 * (ProjectPage) and the live chat (SessionChat) so the stage colours never
 * drift between the two surfaces.
 *
 * The stage VOCABULARY is the backend's: [`Stage`] is generated from
 * `pipeline::Stage` via specta, so a variant added in Rust is a compile error
 * here until it gets a label ([`STAGES`]) and a style ([`MODE_STYLES`], a
 * `Record<StageKey, …>`). Only presentation lives in this file.
 */

import type { Stage, WorkstreamView } from "@/lib/bindings";
import type { DocKind } from "./docs";

export type StageKey = Stage;

export const STAGES = [
  { key: "general", label: "General" },
  { key: "research", label: "Research" },
  { key: "plan", label: "Plan" },
  { key: "implement", label: "Implement" },
  { key: "validate", label: "Validate" },
  { key: "commit", label: "Commit" },
  { key: "ship", label: "Ship" },
  { key: "revise", label: "Revise" },
] as const satisfies readonly { key: StageKey; label: string }[];

/** Stages a user can pick as an agent mode. Commit, Ship and Revise are excluded
 *  on purpose, all are driven by header/PR actions (the "Commit" and "Ship"
 *  buttons and the PR bar's feedback actions), not chosen as a chat mode. They
 *  stay in [`STAGES`] so their icon/colour/label still resolve for their tabs. */
export const MODE_STAGES = STAGES.filter(
  (s) => s.key !== "commit" && s.key !== "ship" && s.key !== "revise",
);

const STAGE_KEYS = new Set<string>(STAGES.map((s) => s.key));

/**
 * SDLC phases the Agents view folders agents by, a coarser rollup of
 * [`Stage`], one folder per phase rather than one per stage (five of the eight
 * stages would otherwise hold a single agent). Display order; "Any stage" is
 * the catch-all a description naming no stage falls into and sorts last.
 */
export const SDLC_PHASES = ["Discover", "Design", "Build", "Verify", "Deliver", "Any stage"] as const;

export type SdlcPhase = (typeof SDLC_PHASES)[number];

/** Catch-all folder for a description that names no stage. */
export const ANY_STAGE: SdlcPhase = "Any stage";

/** Which phase each stage rolls up to. `Record<StageKey, …>` so a stage added
 *  to the Rust enum is a compile error here until it is assigned a phase, the
 *  same guard [`MODE_STYLES`] gets. */
export const STAGE_PHASE: Record<StageKey, SdlcPhase> = {
  general: "Any stage",
  research: "Discover",
  plan: "Design",
  implement: "Build",
  validate: "Verify",
  commit: "Deliver",
  ship: "Deliver",
  revise: "Verify",
};

/**
 * The phase an agent falls into when its description names no stage, keyed by
 * frontmatter `name`. Organization is presentation, so it lives here rather
 * than in the agent files: hyprlayer-cli dropped the `category:` key it used
 * to ship (`feat/named-subagents`), leaving shipped frontmatter to the four
 * keys Claude Code documents. These eleven predate the "Spawn during the
 * <Stage> stage" phrasing and would otherwise all land in [`ANY_STAGE`].
 *
 * An agent written to the convention needs no entry — the description wins
 * (see `phaseFolder`), and adding one here would be dead weight. `Discover`
 * is the rollup for every research/lookup agent, folding in the old Knowledge
 * Graph and Tools categories.
 *
 * `Record<string, …>`, not a closed key union: the name is free text from a
 * file we do not own. Read it through a `Map` (see `NAME_PHASE`), never by
 * bare index.
 */
export const AGENT_PHASE: Record<string, SdlcPhase> = {
  "adversarial-reviewer": "Verify",
  ship: "Deliver",
  archivist: "Discover",
  "codebase-analyzer": "Discover",
  "codebase-locator": "Discover",
  "codebase-pattern-finder": "Discover",
  "jira-searcher": "Discover",
  "jira-ticket-reader": "Discover",
  "thoughts-analyzer": "Discover",
  "thoughts-locator": "Discover",
  "web-search-researcher": "Discover",
};

/**
 * The theme custom-property *name* (never a raw hex, the same rule
 * `Engine.accent_token` follows in `settings.rs`) that colours each phase's
 * folder header and agent dots. "Any stage" carries none on purpose: it is a
 * holding pen, not a phase, and renders muted instead (see `dotColor`).
 */
export const PHASE_COLOR: Record<Exclude<SdlcPhase, "Any stage">, string> = {
  Discover: "--color-research",
  Design: "--color-plan",
  Build: "--color-implement",
  Verify: "--color-validate",
  Deliver: "--color-commit",
};

/** The workstream's stage, already typed by the bindings union. */
export function stageOf(view: WorkstreamView): StageKey {
  return view.item.stage;
}

/** Normalise an arbitrary stage string to a known key (default general).
 *  Still needed for values that DON'T come off the typed wire: localStorage-
 *  persisted tab modes from older app versions. */
export function asStageKey(stage: string): StageKey {
  return (STAGE_KEYS.has(stage) ? stage : "general") as StageKey;
}

export function stageLabel(stage: string): string {
  return STAGES.find((s) => s.key === asStageKey(stage))!.label;
}

/** The next selectable agent stage after `stage`, wrapping around, the order
 *  follows [`MODE_STAGES`] (general → research → plan → implement → validate →
 *  general). Drives the "cycle agent stage" keybinding. A stage outside
 *  [`MODE_STAGES`] (commit/ship/revise, or anything unknown) starts the cycle at
 *  the first mode. */
export function nextModeStage(stage: string): StageKey {
  // Exact match (callers pass a normalized StageKey): a stage outside the cycle
  // lands at index -1, so (-1 + 1) % n = 0, the first mode.
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
 * plain prompt (general chat; ship and revise send prose directives). Mirror of
 * the backend `Stage::command` (crates/hyprlayer-core/src/pipeline.rs) for UI
 * hints only, the backend composes what's actually sent (including any
 * Settings → Prompts override) and returns it for the transcript echo.
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
 * Tailwind class fragments per stage: the active mode button, the inline
 * badge, the composer textarea border, and the standalone icon accent (the
 * text-hue alone, for a colored icon on a neutral chip, see the board tiles).
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
