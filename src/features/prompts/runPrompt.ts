/**
 * Run a custom prompt in a workstream: resolve its effective body (per-project
 * override → global), substitute `{{variables}}` from the worktree (one IPC
 * fetch, skipped when the body has none), and hand the final text to the shared
 * delivery path in `sessionStore`. Built-in prompts don't come through here —
 * Commit/Ship run their stage actions (`commitWorkstream` / `shipWorkstream`)
 * and the backend applies their overrides at launch.
 */

import { commands, type CustomPrompt, type WorkstreamView } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { runPromptTextInWorkstream } from "@/features/session/sessionStore";
import { bodyHasVars, effectiveCustomBody, substitutePromptVars } from "./prompts";

export async function runCustomPrompt(
  view: WorkstreamView,
  prompt: CustomPrompt,
  /** Deliver into this shell tab instead of the focused one (composer chips). */
  targetShellId?: number,
): Promise<void> {
  const body = effectiveCustomBody(prompt, view.item.projectPath);
  const text = bodyHasVars(body)
    ? substitutePromptVars(body, await unwrap(commands.workstreamPromptContext(view.item.id)))
    : body;
  await runPromptTextInWorkstream(view.item.id, text, prompt.name || "Prompt", targetShellId);
}
