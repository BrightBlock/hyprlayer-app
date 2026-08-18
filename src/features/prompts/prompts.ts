/**
 * Prompt library pure core: built-in descriptors, effective-body resolution
 * (global vs per-project override vs stock default), and `{{variable}}` /
 * `{{input}}` substitution. Built-in Commit/Ship bodies are substituted
 * BACKEND-side at launch (`start_stage_session`); the frontend substitutes only
 * what it sends itself (custom prompts, the live-commit re-run). Stock texts come
 * from the bootstrap's `promptDefaults`, never a frontend literal, so the two
 * can't drift.
 */

import type {
  AppSettings,
  CustomPrompt,
  PromptContext,
  PromptDefaults,
} from "@/lib/bindings";
// Shared with the agent library, so the tokens live in lib/domain; re-exported
// here for this feature's existing call sites.
import { PROMPT_VARS } from "@/lib/domain/promptVars";

/** The overridable built-in action prompts. `commandId` is the keyboard-catalog
 *  command that runs them, a built-in's "shortcut" IS that command's binding,
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
    description: "Checkpoint this worktree's changes locally. No push or PR.",
  },
  {
    id: "ship",
    name: "Ship",
    commandId: "ws.ship",
    description: "Commit, push, and open a pull request against the base branch.",
  },
];

export { PROMPT_VARS };

/** An argument slot the user fills in when the prompt runs: `{{input}}`, or
 *  `{{input:Which module?}}` to label the field. */
export type PromptInput = { token: string; label: string };

/** The bare argument token, offered by the library's variable picker. */
export const PROMPT_INPUT_TOKEN = "{{input}}";

const INPUT_RE = /\{\{input(?::([^}]*))?\}\}/g;

/** True when `body` references at least one known `{{variable}}`, the gate for
 *  fetching the (git-diffing) prompt context before a send. */
export function bodyHasVars(body: string): boolean {
  return PROMPT_VARS.some((v) => body.includes(v.token));
}

/** The distinct argument slots in `body`, in order of first appearance. Repeats
 *  of the same token are one field filled once, so `{{input:file}}` can appear
 *  twice in a body and stay a single question. */
export function promptInputs(body: string): PromptInput[] {
  const out: PromptInput[] = [];
  const seen = new Set<string>();
  for (const m of body.matchAll(INPUT_RE)) {
    if (seen.has(m[0])) continue;
    seen.add(m[0]);
    out.push({ token: m[0], label: (m[1] ?? "").trim() });
  }
  return out;
}

/** Single-pass replacement of `pairs`, earliest token first. Mirrors the backend
 *  rule (`pipeline::substitute_vars`): unknown tokens pass through verbatim, and
 *  scanning resumes AFTER each substituted value rather than rescanning it, so
 *  a `{{branch}}` inside a diff hunk, or a `{{diff}}` someone typed into an
 *  argument, survives as literal text. */
function substituteTokens(
  body: string,
  pairs: ReadonlyArray<{ token: string; value: string }>,
): string {
  let out = "";
  let rest = body;
  for (;;) {
    let next: { i: number; token: string; value: string } | null = null;
    for (const p of pairs) {
      const i = rest.indexOf(p.token);
      if (i !== -1 && (next === null || i < next.i)) {
        next = { i, token: p.token, value: p.value };
      }
    }
    if (next === null) return out + rest;
    out += rest.slice(0, next.i) + next.value;
    rest = rest.slice(next.i + next.token.length);
  }
}

/** Replace the known `{{variable}}` tokens with their worktree values. */
export function substitutePromptVars(body: string, ctx: PromptContext): string {
  return substituteTokens(
    body,
    PROMPT_VARS.map((v) => ({ token: v.token, value: ctx[v.key] })),
  );
}

/** A custom prompt's final text: worktree variables and user arguments resolved
 *  in ONE pass, so neither kind of token can expand out of the other's value.
 *  `ctx` is null when the body references no variable (no git fetch was needed);
 *  an argument with no answer resolves to empty. */
export function resolvePromptBody(
  body: string,
  ctx: PromptContext | null,
  args: Record<string, string>,
): string {
  return substituteTokens(body, [
    ...(ctx ? PROMPT_VARS.map((v) => ({ token: v.token, value: ctx[v.key] })) : []),
    ...promptInputs(body).map((i) => ({ token: i.token, value: args[i.token] ?? "" })),
  ]);
}

/** The user's custom prompts, tolerant of a sparse settings object. */
export function customPrompts(settings: AppSettings | null | undefined): CustomPrompt[] {
  return settings?.prompts?.custom ?? [];
}

/** A custom prompt's body for `projectPath`, its override there, else global. */
export function effectiveCustomBody(prompt: CustomPrompt, projectPath: string | null): string {
  if (projectPath) {
    const override = prompt.overrides?.[projectPath];
    if (override !== undefined) return override;
  }
  return prompt.body ?? "";
}

/** Whether this prompt has anything to send for `projectPath`. An empty
 *  effective body would run as a silent no-op, so every surface that offers
 *  prompts, chips, palette, `/slug` autocomplete, chords, gates on this. */
export function isRunnablePrompt(prompt: CustomPrompt, projectPath: string | null): boolean {
  return effectiveCustomBody(prompt, projectPath).trim().length > 0;
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

/** The `/token` a custom prompt answers to in the composer: its name lowercased,
 *  with runs of anything else collapsed to `-`. Empty when the name has no usable
 *  character, that prompt simply isn't offered as a command. */
export function promptSlug(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

/** A composer draft that opens with a custom prompt's `/slug`. */
export type ComposerPromptMatch = {
  prompt: CustomPrompt;
  /** Everything typed after the slug, the prompt's argument text. */
  typed: string;
};

/** The prompts a composer will expand, keyed by slug. Built once and used for
 *  both the dropdown and the match, so what gets offered is exactly what gets
 *  run: a slug a real claude command owns, an unusable name, an empty body for
 *  this project, or a duplicate of a slug already claimed never lands here. */
export function offeredPrompts(
  prompts: CustomPrompt[],
  projectPath: string | null,
  takenByHarness: ReadonlySet<string>,
): Map<string, CustomPrompt> {
  const out = new Map<string, CustomPrompt>();
  for (const p of prompts) {
    const slug = promptSlug(p.name ?? "");
    if (!slug || takenByHarness.has(slug) || out.has(slug)) continue;
    if (!isRunnablePrompt(p, projectPath)) continue;
    out.set(slug, p);
  }
  return out;
}

/** Match a draft whose FIRST token is an offered prompt slug; a `/token` further
 *  in is ordinary text, matching how claude reads its own commands. */
export function matchComposerPrompt(
  draft: string,
  offered: ReadonlyMap<string, CustomPrompt>,
): ComposerPromptMatch | null {
  const m = /^\/([a-z0-9-]+)(?:\s+([\s\S]*))?$/.exec(draft.trim());
  if (!m) return null;
  const prompt = offered.get(m[1]);
  return prompt ? { prompt, typed: (m[2] ?? "").trim() } : null;
}

/** Display name for an argument slot, an unlabelled `{{input}}` still needs
 *  one. Callers style it; the dialog uppercases, the composer ghost doesn't. */
export function promptInputLabel(input: PromptInput): string {
  return input.label || "argument";
}

/** The lone argument a matched `/slug` draft still needs, or null when it's
 *  ready to send. Only single-slot bodies qualify: the composer line can fill
 *  one argument, so asking for it in a dialog would be asking someone standing
 *  in a text box to type into another one. Two or more slots can't be expressed
 *  on one line and still open the dialog. */
export function pendingComposerArg(
  match: ComposerPromptMatch,
  projectPath: string | null,
): PromptInput | null {
  if (match.typed) return null;
  const inputs = promptInputs(effectiveCustomBody(match.prompt, projectPath));
  return inputs.length === 1 ? inputs[0] : null;
}

/** Put `/slug ` at the head of a composer draft. A command already sitting there
 *  is swapped, so a second chip click replaces the first instead of nesting;
 *  anything else already typed is kept and becomes the argument. The caret
 *  belongs at `slug.length + 2`, just past the inserted token. */
export function prefillSlashCommand(draft: string, slug: string): string {
  const rest = draft.replace(/^\s*\/[a-z0-9-]*\s*/, "");
  return rest ? `/${slug} ${rest}` : `/${slug} `;
}

/** First line of a body, for palette/list descriptions. */
export function promptFirstLine(body: string): string {
  return body.split("\n", 1)[0] ?? "";
}
