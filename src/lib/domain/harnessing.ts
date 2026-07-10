import type { AppSettings, CatalogProvider, Harness, PermissionMode } from "@/lib/bindings";
import { effectiveEffort, modelOption } from "./models";
import { permissionModeOption } from "./permissions";

/**
 * The agent harnesses a session can run on. `key` is typed by the bindings'
 * [`Harness`] union (generated from core's enum) — what `start_stage_session`
 * expects in `opts.harness` and what the session row stores.
 */
export type HarnessOption = { key: Harness; label: string };

export const HARNESSES: HarnessOption[] = [
  { key: "claude", label: "Claude Code" },
  { key: "opencode", label: "OpenCode" },
];

export const DEFAULT_HARNESS_KEY = "claude";

/** The harnesses selectable as the app-wide default (Settings → General). Unlike
 *  [`HARNESSES`], this lists Ollama as its own choice even though it runs on the
 *  OpenCode harness (mapped by [`effectiveHarnessKey`]) — so users pick it the way
 *  they think of it. */
export const DEFAULT_HARNESS_OPTIONS: { key: string; label: string }[] = [
  { key: "claude", label: "Claude Code" },
  { key: "opencode", label: "OpenCode" },
  { key: "ollama", label: "Ollama" },
];

export function harnessOption(key: string | undefined | null): HarnessOption {
  return HARNESSES.find((h) => h.key === key) ?? HARNESSES[0];
}

/** The actual harness a stored default-harness key runs on: Claude stays Claude,
 *  while OpenCode *and* Ollama both run on the OpenCode harness (Ollama's models
 *  are `ollama/<id>` OpenCode ids). Null/unknown ⇒ Claude. Use this to turn the
 *  three-way `providers.defaultHarness` setting into a real [`HarnessOption`] key. */
export function effectiveHarnessKey(defaultHarness: string | null | undefined): Harness {
  return defaultHarness === "opencode" || defaultHarness === "ollama" ? "opencode" : "claude";
}

/** The launch config for the action-driven agents (Commit / Ship / Revise),
 *  resolved from settings. `model` is a model KEY when `harness === "claude"`
 *  (see models.ts), or a `"provider/model"` id when `"opencode"`; `null` means
 *  "let the harness pick its own default". `effort` is a Claude effort key, or
 *  `null` for OpenCode (which has no `--effort`). */
export type ActionAgentDefaults = {
  harness: Harness;
  model: string | null;
  effort: string | null;
};

/** Resolve the Commit/Ship/Revise agents' defaults from settings: the default
 *  harness (Claude / OpenCode / Ollama), and — within it — the cheaper model +
 *  reasoning configured on that harness's Settings tab. Each harness owns its own
 *  commit model, so switching the default never leaves an unrunnable model behind;
 *  each falls back sensibly (Sonnet/medium for Claude, the harness's default or
 *  first-selected model otherwise) so an action always names a runnable model. */
export function actionAgentDefaults(
  settings: AppSettings | null | undefined,
): ActionAgentDefaults {
  const providers = settings?.providers;
  const dh = providers?.defaultHarness ?? "claude";
  if (dh === "ollama") {
    // Ollama runs through the OpenCode harness; its models are `ollama/<id>` ids.
    const ollama = providers?.ollama;
    const first = ollama?.selectedModels?.[0];
    const model =
      ollama?.commitModel ?? ollama?.defaultModel ?? (first ? `ollama/${first}` : null);
    return { harness: "opencode", model: model ?? null, effort: null };
  }
  if (dh === "opencode") {
    const oc = providers?.opencode;
    const model = oc?.commitModel ?? oc?.defaultModel ?? oc?.selectedModels?.[0] ?? null;
    return { harness: "opencode", model, effort: null };
  }
  const claude = providers?.claude;
  return {
    harness: "claude",
    model: claude?.commitModel ?? "sonnet",
    effort: claude?.commitEffort ?? "medium",
  };
}

/** The `startSession` model/effort args for resolved [`ActionAgentDefaults`]:
 *  Claude translates its model KEY to the `--model` flag (Sonnet's key and flag
 *  coincide; `"opus"` → the null default; a custom id → itself); OpenCode passes
 *  the `provider/model` id through and carries no effort. */
export function actionLaunchArgs(d: ActionAgentDefaults): {
  harness: Harness;
  model: string | null;
  effort: string | null;
} {
  if (d.harness === "claude") {
    return {
      harness: "claude",
      model: d.model ? modelOption(d.model).flag : null,
      effort: d.effort,
    };
  }
  return { harness: d.harness, model: d.model, effort: null };
}

/** A shell tab's persisted composer picks, when starting a session on its
 *  behalf from outside SessionChat (e.g. running a prompt into an unstarted
 *  tab). All optional — absent fields fall back to the settings defaults. */
export type ShellLaunchPicks = {
  harness?: string;
  model?: string;
  effort?: string;
  opencodeModel?: string;
  permissionMode?: string;
};

/** The `startSession` launch args for a NEW plain chat, resolved exactly the
 *  way SessionChat's composer does for a fresh tab: the tab's own persisted
 *  picks first, then the configured defaults (Settings → Harnesses, Claude's
 *  `defaultPlanMode`), then the built-ins. The prompt-run path uses this so a
 *  prompt fired at an unstarted tab launches on the same config pressing Enter
 *  there would — including the permission mode. */
export function newChatLaunchArgs(
  settings: AppSettings | null | undefined,
  shell: ShellLaunchPicks | null | undefined,
): {
  harness: Harness;
  model: string | null;
  effort: string | null;
  permissionMode: PermissionMode | null;
} {
  const providers = settings?.providers;
  const claude = providers?.claude;
  const permissionMode = permissionModeOption(
    shell?.permissionMode ?? (claude?.defaultPlanMode ? "plan" : undefined),
  ).flag;
  const harnessKey = shell?.harness ?? effectiveHarnessKey(providers?.defaultHarness);
  if (harnessKey === "opencode") {
    const oc = providers?.opencode;
    const ollama = providers?.ollama;
    const first = ollama?.selectedModels?.[0];
    const model =
      shell?.opencodeModel ??
      oc?.defaultModel ??
      ollama?.defaultModel ??
      oc?.selectedModels?.[0] ??
      (first ? `ollama/${first}` : null);
    return { harness: "opencode", model: model ?? null, effort: null, permissionMode };
  }
  const model = modelOption(shell?.model ?? claude?.defaultModel ?? undefined);
  const effort = effectiveEffort(shell?.effort ?? claude?.defaultEffort ?? undefined, model);
  return { harness: "claude", model: model.flag, effort: effort.key, permissionMode };
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
