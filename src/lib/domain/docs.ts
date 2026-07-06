/**
 * The doc kinds an agent stage produces/attaches (the research → plan →
 * handoff pipeline vocabulary). Shared by the session transcript, the stage
 * model, and the workstreams changes panel; the full library taxonomy
 * (note/pr/…) lives in `features/library/constants.ts` until it is generated
 * from a core enum.
 */
export type DocKind = "research" | "plan" | "handoff";
