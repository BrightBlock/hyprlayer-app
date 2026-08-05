/**
 * Turning a custom prompt into an agent turn. `resolveCustomPrompt` is the one
 * place that decides what text gets sent: it picks the effective body
 * (per-project override → global), collects any `{{input}}` arguments, and
 * substitutes those together with the `{{variables}}` read from the worktree
 * (one IPC fetch, skipped when the body has none). The prompt library's own
 * surfaces go through `runCustomPrompt`; the composer's `/slug` expansion calls
 * the resolver directly and sends the result itself. Built-in prompts come
 * through neither — Commit/Ship run their stage actions and the backend applies
 * their overrides at launch.
 */

import { commands, type CustomPrompt, type WorkstreamView } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { runPromptTextInWorkstream } from "@/features/session/sessionStore";
import {
  bodyHasVars,
  effectiveCustomBody,
  promptInputs,
  resolvePromptBody,
} from "./prompts";
import { askPromptArgs } from "./promptsStore";

const TITLE_ARG_MAX = 40;

/** Two runs of one prompt differ only by their argument, so carry it into the
 *  transcript card and the tab title. */
function runTitle(name: string, detail: string): string {
  if (!detail) return name;
  const short =
    detail.length > TITLE_ARG_MAX ? `${detail.slice(0, TITLE_ARG_MAX - 1)}…` : detail;
  return `${name}: ${short}`;
}

export type ResolvedPrompt = { text: string; title: string };

/**
 * The final turn text for `prompt`, asking for arguments when it needs them.
 * Null when the user cancels the ask.
 *
 * `typed` is what followed `/slug` in the composer: it fills a lone `{{input}}`
 * outright, seeds the first field when the dialog has to open anyway, and — for
 * a body with no slot at all — rides along as a trailing line, so text the user
 * typed never silently disappears.
 */
export async function resolveCustomPrompt(
  workstreamId: string,
  projectPath: string | null,
  prompt: CustomPrompt,
  typed = "",
): Promise<ResolvedPrompt | null> {
  const body = effectiveCustomBody(prompt, projectPath);
  const name = prompt.name || "Prompt";
  const inputs = promptInputs(body);
  const seed = typed && inputs.length > 0 ? { [inputs[0].token]: typed } : {};

  // Ask before the context fetch — that one runs a git diff, wasted if cancelled.
  const asked =
    inputs.length > 1 || (inputs.length === 1 && !typed)
      ? await askPromptArgs(name, inputs, seed)
      : seed;
  if (asked === null) return null;

  const ctx = bodyHasVars(body)
    ? await unwrap(commands.workstreamPromptContext(workstreamId))
    : null;
  const resolved = resolvePromptBody(body, ctx, asked);
  const text = inputs.length === 0 && typed ? `${resolved}\n\n${typed}` : resolved;

  const detail = inputs.map((i) => asked[i.token]?.trim()).find((v) => v) ?? typed;
  return { text, title: runTitle(name, detail) };
}

/** Run a custom prompt from the library's own surfaces (⌘K palette, composer
 *  chips, prompt shortcuts) via the shared delivery path in `sessionStore`. */
export async function runCustomPrompt(
  view: WorkstreamView,
  prompt: CustomPrompt,
  /** Deliver into this shell tab instead of the focused one (composer chips). */
  targetShellId?: number,
): Promise<void> {
  const resolved = await resolveCustomPrompt(view.item.id, view.item.projectPath, prompt);
  if (!resolved) return;
  await runPromptTextInWorkstream(view.item.id, resolved.text, resolved.title, targetShellId);
}
