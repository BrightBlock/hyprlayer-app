
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

export const MODE_STAGES = STAGES.filter(
  (s) => s.key !== "commit" && s.key !== "ship" && s.key !== "revise",
);

const STAGE_KEYS = new Set<string>(STAGES.map((s) => s.key));

export const SDLC_PHASES = ["Discover", "Design", "Build", "Verify", "Deliver", "Any stage"] as const;

export type SdlcPhase = (typeof SDLC_PHASES)[number];

export const ANY_STAGE: SdlcPhase = "Any stage";

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

export const PHASE_COLOR: Record<Exclude<SdlcPhase, "Any stage">, string> = {
  Discover: "--color-research",
  Design: "--color-plan",
  Build: "--color-implement",
  Verify: "--color-validate",
  Deliver: "--color-commit",
};

export function stageOf(view: WorkstreamView): StageKey {
  return view.item.stage;
}

export function asStageKey(stage: string): StageKey {
  return (STAGE_KEYS.has(stage) ? stage : "general") as StageKey;
}

export function stageLabel(stage: string): string {
  return STAGES.find((s) => s.key === asStageKey(stage))!.label;
}

export function nextModeStage(stage: string): StageKey {
  const i = MODE_STAGES.findIndex((s) => s.key === stage);
  return MODE_STAGES[(i + 1) % MODE_STAGES.length].key;
}

export function suggestedDocKind(stage: string): DocKind | null {
  switch (asStageKey(stage)) {
    case "plan":
      return "research";
    case "implement":
      return "plan";
    case "validate":
      return "plan";
    default:
      return null;
  }
}

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
