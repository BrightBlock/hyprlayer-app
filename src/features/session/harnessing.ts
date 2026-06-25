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
