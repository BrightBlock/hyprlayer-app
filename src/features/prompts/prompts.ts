/**
 * Prompt library pure core: built-in descriptors, effective-body resolution
 * (global vs per-project override vs stock default), and `{{variable}}`
 * substitution. Built-in Commit/Ship bodies are substituted BACKEND-side at
 * launch (`start_stage_session`); the frontend substitutes only what it sends
 * itself (custom prompts, the live-commit re-run). Stock texts come from the
 * bootstrap's `promptDefaults`, never a frontend literal, so the two can't drift.
 */

import type {
  AppSettings,
  CustomPrompt,
  PromptContext,
  PromptDefaults,
} from "@/lib/bindings";

/** The overridable built-in action prompts. `commandId` is the keyboard-catalog
 *  command that runs them — a built-in's "shortcut" IS that command's binding,
 *  so recording one from the prompt editor writes the same override Settings →
 *  Keyboard shows. */
export type BuiltinPromptId = "commit" | "ship";

export const BUILTIN_PROMPTS: ReadonlyArray<{
  id: BuiltinPromptId;
  name: string;
  commandId: string;
  description: string;
}> = [
  {
    id: "commit",
    name: "Commit",
    commandId: "ws.commit",
    description: "Checkpoint this worktree's changes locally — no push or PR.",
  },
  {
    id: "ship",
    name: "Ship",
    commandId: "ws.ship",
    description: "Commit, push, and open a pull request against the base branch.",
  },
];

/** The variables a prompt body may reference. Values resolve from the focused
 *  workstream's worktree when the prompt runs (`workstream_prompt_context`). */
export const PROMPT_VARS: ReadonlyArray<{
  token: string;
  key: keyof PromptContext;
  desc: string;
}> = [
  { token: "{{branch}}", key: "branch", desc: "Current worktree branch" },
  { token: "{{base_branch}}", key: "baseBranch", desc: "PR target branch" },
  { token: "{{changed_files}}", key: "changedFiles", desc: "Working-tree change summary" },
  { token: "{{diff}}", key: "diff", desc: "Working-tree diff vs HEAD (truncated)" },
  { token: "{{last_commit}}", key: "lastCommit", desc: "Most recent commit subject" },
];

/** True when `body` references at least one known `{{variable}}` — the gate for
 *  fetching the (git-diffing) prompt context before a send. */
export function bodyHasVars(body: string): boolean {
  return PROMPT_VARS.some((v) => body.includes(v.token));
}

/** Replace the known `{{variable}}` tokens. Mirrors the backend rule
 *  (`pipeline::substitute_vars`): fixed token set, unknown tokens pass through
 *  verbatim, and substitution is single-pass over the ORIGINAL body — values
 *  are never rescanned, so token-shaped text inside e.g. the `{{diff}}` value
 *  survives verbatim. */
export function substitutePromptVars(body: string, ctx: PromptContext): string {
  let out = "";
  let rest = body;
  for (;;) {
    let next: { i: number; token: string; value: string } | null = null;
    for (const v of PROMPT_VARS) {
      const i = rest.indexOf(v.token);
      if (i !== -1 && (next === null || i < next.i)) {
        next = { i, token: v.token, value: ctx[v.key] };
      }
    }
    if (next === null) return out + rest;
    out += rest.slice(0, next.i) + next.value;
    rest = rest.slice(next.i + next.token.length);
  }
}

/** The user's custom prompts, tolerant of a sparse settings object. */
export function customPrompts(settings: AppSettings | null | undefined): CustomPrompt[] {
  return settings?.prompts?.custom ?? [];
}

/** A custom prompt's body for `projectPath` — its override there, else global. */
export function effectiveCustomBody(prompt: CustomPrompt, projectPath: string | null): string {
  if (projectPath) {
    const override = prompt.overrides?.[projectPath];
    if (override !== undefined) return override;
  }
  return prompt.body ?? "";
}

/** A built-in prompt's effective body: the Settings → Prompts override, else
 *  the stock text from the bootstrap's `promptDefaults`. */
export function effectiveBuiltinBody(
  settings: AppSettings | null | undefined,
  defaults: PromptDefaults | null | undefined,
  id: BuiltinPromptId,
): string {
  return settings?.prompts?.builtinBodies?.[id] ?? defaults?.[id] ?? "";
}

/** Whether a built-in's body has been customized away from stock. */
export function isBuiltinModified(
  settings: AppSettings | null | undefined,
  defaults: PromptDefaults | null | undefined,
  id: BuiltinPromptId,
): boolean {
  const override = settings?.prompts?.builtinBodies?.[id];
  return override !== undefined && override !== (defaults?.[id] ?? "");
}

/** First line of a body, for palette/list descriptions. */
export function promptFirstLine(body: string): string {
  return body.split("\n", 1)[0] ?? "";
}
