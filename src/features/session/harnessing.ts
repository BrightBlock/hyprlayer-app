import type { CatalogProvider } from "../../lib/bindings";

/**
 * The agent harnesses a session can run on. `key` is what the backend's
 * `start_stage_session` expects in `opts.harness` and what the session row
 * stores. Mirrors the small option-list shape of `models.ts`.
 */
export type HarnessOption = { key: string; label: string };

export const HARNESSES: HarnessOption[] = [
  { key: "claude", label: "Claude Code" },
  { key: "opencode", label: "OpenCode" },
];

export const DEFAULT_HARNESS_KEY = "claude";

export function harnessOption(key: string | undefined | null): HarnessOption {
  return HARNESSES.find((h) => h.key === key) ?? HARNESSES[0];
}

/** OpenCode Zen (and similar gateways) expose anonymous `-free` models that
 *  authenticate without an API key. Detect them by the conventional `-free` id
 *  suffix so the pickers can surface them even when the provider has no stored
 *  credential. Takes a bare model id (e.g. `glm-5-free`), not a `provider/model`. */
export function isFreeOpencodeModel(modelId: string): boolean {
  return modelId.toLowerCase().endsWith("-free");
}

/** The friendly model + provider names behind an OpenCode `provider/model` id. */
export type OpencodeModelName = { model: string; provider: string };

/** Index the models.dev catalog by `provider/model` id, so the composer can show
 *  a model's friendly name instead of its long id. */
export function opencodeModelNames(
  providers: CatalogProvider[],
): Map<string, OpencodeModelName> {
  const map = new Map<string, OpencodeModelName>();
  for (const p of providers) {
    for (const m of p.models) {
      map.set(`${p.id}/${m.id}`, { model: m.name, provider: p.name });
    }
  }
  return map;
}

/** A friendly label for an OpenCode `provider/model` id: the catalog's model name
 *  when known, else the last id segment (e.g. `kimi-k2p7-code-fast`). */
export function opencodeModelLabel(
  id: string,
  names: Map<string, OpencodeModelName>,
): string {
  return names.get(id)?.model ?? id.split("/").pop() ?? id;
}

/**
 * The underlying provider behind a model pick — what the upstream API endpoint is
 * keyed on. OpenCode model ids are `provider/model…`, so the provider is the first
 * segment (`openrouter`, `opencode`, `ollama`, …); every Claude model shares the
 * single `claude` provider. Returns `null` only for an OpenCode harness with no
 * model resolved yet.
 */
export function modelProvider(
  harnessKey: string,
  opencodeModelId: string | null | undefined,
): string | null {
  if (harnessKey === "opencode") return opencodeModelId ? opencodeModelId.split("/")[0] || null : null;
  return "claude";
}

/**
 * The provider a started session is locked to, or `null` when nothing is locked
 * (the session hasn't started, or its OpenCode model isn't resolved yet).
 *
 * A started session may only switch models *within* this provider. Switching
 * across providers mid-session replays the session's accumulated tool history to a
 * different endpoint, and a stricter provider (e.g. OpenCode Zen / DeepSeek)
 * rejects OpenCode's synthesized `functions.{tool}:{n}` ids as duplicates
 * ("Duplicate value for 'tool_call_id'…"), hard-failing the turn. Claude is immune
 * because all its models are one provider — so model switching there is seamless;
 * this keeps OpenCode to the same rule (same-provider switches only).
 */
export function lockedSessionProvider(
  started: boolean,
  harnessKey: string,
  opencodeModelId: string | null | undefined,
): string | null {
  return started ? modelProvider(harnessKey, opencodeModelId) : null;
}
