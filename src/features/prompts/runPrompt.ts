
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

function runTitle(name: string, detail: string): string {
  if (!detail) return name;
  const short =
    detail.length > TITLE_ARG_MAX ? `${detail.slice(0, TITLE_ARG_MAX - 1)}…` : detail;
  return `${name}: ${short}`;
}

export type ResolvedPrompt = { text: string; title: string };

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

export async function runCustomPrompt(
  view: WorkstreamView,
  prompt: CustomPrompt,
  targetShellId?: number,
): Promise<void> {
  const resolved = await resolveCustomPrompt(view.item.id, view.item.projectPath, prompt);
  if (!resolved) return;
  await runPromptTextInWorkstream(view.item.id, resolved.text, resolved.title, targetShellId);
}
