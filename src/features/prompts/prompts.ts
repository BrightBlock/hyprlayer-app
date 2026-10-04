
import type {
  AppSettings,
  CustomPrompt,
  PromptContext,
  PromptDefaults,
  WorkstreamView,
  WorktreeStatus,
} from "@/lib/bindings";
import { isFolderWorkstream, shortBranch } from "@/lib/domain/projects";
import { PROMPT_VARS } from "@/lib/domain/promptVars";
import { plural } from "@/lib/format";

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

export type PromptFilter = "all" | "pinned" | "overrides";

export const PROMPT_FILTERS: ReadonlyArray<{ key: PromptFilter; label: string }> = [
  { key: "all", label: "All prompts" },
  { key: "pinned", label: "Pinned to composer" },
  { key: "overrides", label: "Project overrides" },
];

export function promptFilter(raw: string | undefined): PromptFilter {
  return PROMPT_FILTERS.find((f) => f.key === raw)?.key ?? "all";
}

export function filterCustomPrompts(prompts: CustomPrompt[], filter: PromptFilter): CustomPrompt[] {
  switch (filter) {
    case "pinned":
      return prompts.filter((p) => p.pinned);
    case "overrides":
      return prompts.filter((p) => Object.keys(p.overrides ?? {}).length > 0);
    default:
      return prompts;
  }
}

export type PromptInput = { token: string; label: string };

export const PROMPT_INPUT_TOKEN = "{{input}}";

const INPUT_RE = /\{\{input(?::([^}]*))?\}\}/g;

export function bodyHasVars(body: string): boolean {
  return PROMPT_VARS.some((v) => body.includes(v.token));
}

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

export function substitutePromptVars(body: string, ctx: PromptContext): string {
  return substituteTokens(
    body,
    PROMPT_VARS.map((v) => ({ token: v.token, value: ctx[v.key] })),
  );
}

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

export function customPrompts(settings: AppSettings | null | undefined): CustomPrompt[] {
  return settings?.prompts?.custom ?? [];
}

export function effectiveCustomBody(prompt: CustomPrompt, projectPath: string | null): string {
  if (projectPath) {
    const override = prompt.overrides?.[projectPath];
    if (override !== undefined) return override;
  }
  return prompt.body ?? "";
}

export function isRunnablePrompt(prompt: CustomPrompt, projectPath: string | null): boolean {
  return effectiveCustomBody(prompt, projectPath).trim().length > 0;
}

export function effectiveBuiltinBody(
  settings: AppSettings | null | undefined,
  defaults: PromptDefaults | null | undefined,
  id: BuiltinPromptId,
): string {
  return settings?.prompts?.builtinBodies?.[id] ?? defaults?.[id] ?? "";
}

export function isBuiltinModified(
  settings: AppSettings | null | undefined,
  defaults: PromptDefaults | null | undefined,
  id: BuiltinPromptId,
): boolean {
  const override = settings?.prompts?.builtinBodies?.[id];
  return override !== undefined && override !== (defaults?.[id] ?? "");
}

export function promptSlug(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

export type ComposerPromptMatch = {
  prompt: CustomPrompt;
  typed: string;
};

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

export function matchComposerPrompt(
  draft: string,
  offered: ReadonlyMap<string, CustomPrompt>,
): ComposerPromptMatch | null {
  const m = /^\/([a-z0-9-]+)(?:\s+([\s\S]*))?$/.exec(draft.trim());
  if (!m) return null;
  const prompt = offered.get(m[1]);
  return prompt ? { prompt, typed: (m[2] ?? "").trim() } : null;
}

export function promptInputLabel(input: PromptInput): string {
  return input.label || "argument";
}

export function pendingComposerArg(
  match: ComposerPromptMatch,
  projectPath: string | null,
): PromptInput | null {
  if (match.typed) return null;
  const inputs = promptInputs(effectiveCustomBody(match.prompt, projectPath));
  return inputs.length === 1 ? inputs[0] : null;
}

export function prefillSlashCommand(draft: string, slug: string): string {
  const rest = draft.replace(/^\s*\/[a-z0-9-]*\s*/, "");
  return rest ? `/${slug} ${rest}` : `/${slug} `;
}

export function promptFirstLine(body: string): string {
  return body.split("\n", 1)[0] ?? "";
}

export function previewArgs(body: string): Record<string, string> {
  return Object.fromEntries(promptInputs(body).map((i) => [i.token, `‹${promptInputLabel(i)}›`]));
}

export function previewCandidates(views: WorkstreamView[]): WorkstreamView[] {
  return views
    .filter((v) => v.item.status !== "archived" && !isFolderWorkstream(v))
    .sort((a, b) => (b.item.updatedAt ?? 0) - (a.item.updatedAt ?? 0));
}

export function defaultPreviewTarget(
  candidates: WorkstreamView[],
  scopeProject: string | null,
): WorkstreamView | null {
  return (
    (scopeProject && candidates.find((v) => v.item.projectPath === scopeProject)) ||
    candidates[0] ||
    null
  );
}

export function worktreeStatusLine(git: WorktreeStatus, baseBranch: string): string {
  const line = `${plural(git.dirty + git.untracked, "file")} changed · ${plural(git.aheadOfBase, "commit")} ahead of ${shortBranch(baseBranch)}`;
  return git.behind > 0 ? `${line} · ${git.behind} behind upstream` : line;
}

export function usedVars(body: string): typeof PROMPT_VARS {
  return PROMPT_VARS.filter((v) => body.includes(v.token));
}
