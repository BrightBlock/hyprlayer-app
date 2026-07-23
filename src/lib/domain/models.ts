/**
 * The model and reasoning-effort choices offered in the composer. Both map to
 * `claude` CLI flags passed at spawn time (`--model`, `--effort`) — see the
 * driver's `SpawnConfig` (crates/hyprlayer-core/src/claude/driver.rs).
 *
 * Selections are fixed when the session spawns; changing them only affects the
 * next session you start.
 */

/** A selectable model. `flag` is the `--model` value, or null to leave it to
 *  claude's own default (which is what gives the 1M-context Opus today). */
export type ModelOption = {
  key: string;
  label: string;
  /** A short qualifier shown after the label (e.g. the context window). */
  hint?: string;
  flag: string | null;
  /** Context-window size in tokens — the meter's denominator. Taken from the
   *  selected option, not the model id claude reports (opus reports the base
   *  `claude-opus-4-8` until context crosses 200k, then `[1m]`, which would make
   *  the window flip mid-session). */
  contextWindow: number;
};

/**
 * Mirrors the models the `claude` CLI's own `/model` picker offers (extracted
 * from the binary — the CLI exposes no command to enumerate them). The picker's
 * `value` is exactly our `flag`; the `[1m]` suffix variants unlock the 1M
 * context window. This list is CLI-version-specific and must be refreshed when
 * `claude` adds models — the composer's "Custom…" field is the escape hatch for
 * anything not listed here (e.g. a dated id like `claude-opus-4-7-20...`).
 */
export const MODELS: ModelOption[] = [
  // Latest aliases (the default, Opus, leaves --model unset). The `sonnet`
  // aliases now resolve to Sonnet 5 in the CLI, so only their labels change.
  { key: "opus", label: "Opus 4.8", hint: "1M", flag: null, contextWindow: 1_000_000 },
  { key: "sonnet", label: "Sonnet 5", flag: "sonnet", contextWindow: 200_000 },
  { key: "sonnet-1m", label: "Sonnet 5", hint: "1M", flag: "sonnet[1m]", contextWindow: 1_000_000 },
  { key: "haiku", label: "Haiku 4.5", flag: "haiku", contextWindow: 200_000 },
  { key: "fable", label: "Fable 5", hint: "1M", flag: "fable", contextWindow: 1_000_000 },
  // Previous Sonnet/Opus versions (full ids — these have no short alias).
  { key: "sonnet-4-6", label: "Sonnet 4.6", flag: "claude-sonnet-4-6", contextWindow: 200_000 },
  { key: "sonnet-4-6-1m", label: "Sonnet 4.6", hint: "1M", flag: "claude-sonnet-4-6[1m]", contextWindow: 1_000_000 },
  { key: "opus-4-7", label: "Opus 4.7", flag: "claude-opus-4-7", contextWindow: 200_000 },
  { key: "opus-4-7-1m", label: "Opus 4.7", hint: "1M", flag: "claude-opus-4-7[1m]", contextWindow: 1_000_000 },
  { key: "opus-4-6", label: "Opus 4.6", flag: "claude-opus-4-6", contextWindow: 200_000 },
  { key: "opus-4-6-1m", label: "Opus 4.6", hint: "1M", flag: "claude-opus-4-6[1m]", contextWindow: 1_000_000 },
  { key: "opus-4-1", label: "Opus 4.1", flag: "claude-opus-4-1", contextWindow: 200_000 },
];

export const DEFAULT_MODEL_KEY = "opus";

/** Resolve a stored model key to its option. The presets above are matched by
 *  key; an empty/undefined key falls back to the default (Opus). Any other
 *  non-empty value is treated as a CUSTOM model the user typed — an alias
 *  (`haiku`) or full id (`claude-opus-4-7`) the CLI's `--model` accepts. For a
 *  custom value the string IS both the flag and its own label, since we can't
 *  know a friendly name for an arbitrary id. */
export function modelOption(key: string | undefined): ModelOption {
  if (!key) return MODELS[0];
  const preset = MODELS.find((m) => m.key === key);
  if (preset) return preset;
  return {
    key,
    label: key,
    flag: key,
    // Full ids carry an explicit `[1m]` suffix when they unlock the 1M window;
    // everything else is the standard 200k. The session's init event is the
    // source of truth, but this keeps the meter sane before the model reports.
    contextWindow: key.includes("[1m]") ? 1_000_000 : 200_000,
  };
}

/** The context window the Anthropic Models API reports for a session's RESOLVED
 *  model id (the id `claude` echoes in its `init` event, e.g. `claude-fable-5` or
 *  `claude-opus-4-8[1m]`), or `undefined` when the fetched catalog doesn't cover
 *  it — offline, logged out, an OpenCode/Ollama id, or a model predating the
 *  API's `max_input_tokens` field. This is the live source of truth for the meter
 *  denominator; callers fall back to the static [`ModelOption.contextWindow`]
 *  table when it returns `undefined`. The catalog keys on bare ids, so any
 *  `[…]` variant suffix (`[1m]`) is stripped before lookup. */
export function apiContextWindow(
  resolvedModelId: string | null | undefined,
  windows: ReadonlyMap<string, number>,
): number | undefined {
  if (!resolvedModelId) return undefined;
  return windows.get(resolvedModelId.replace(/\[[^\]]*\]$/, ""));
}

/** The explicit `--model` value to use when RESUMING a live session onto this
 *  model. `claude --resume` keeps the session's original model when `--model` is
 *  omitted, so switching TO the default-Opus entry (whose launch `flag` is null)
 *  mid-conversation needs an explicit id. Opus 4.8 is natively 1M, so
 *  `claude-opus-4-8` matches what the null default launches. */
export function resumeModelFlag(model: ModelOption): string {
  return model.flag ?? "claude-opus-4-8";
}

/** The full list of preset model keys, in display order. */
export const ALL_MODEL_KEYS = MODELS.map((m) => m.key);

/** Set form of [`ALL_MODEL_KEYS`] for O(1) "is this a preset?" membership tests. */
export const ALL_MODEL_KEY_SET = new Set(ALL_MODEL_KEYS);

/**
 * Canonical ordering for a "visible models" list: presets in [`MODELS`] order,
 * then custom ids in first-seen order, de-duplicated and emptied of falsy keys.
 * The single source of truth for normalization — both the Settings UI (on write)
 * and the composer (on read) run through it, so a stored list that predates this
 * rule still renders tidily.
 */
export function normalizeVisibleModelKeys(keys: readonly string[]): string[] {
  // `[...seen]` preserves first-occurrence order, so customs keep insertion order.
  const seen = new Set(keys.filter(Boolean));
  return [
    ...ALL_MODEL_KEYS.filter((key) => seen.has(key)),
    ...[...seen].filter((key) => !ALL_MODEL_KEY_SET.has(key)),
  ];
}

/**
 * The visible model keys to offer, given the user's "visible models" setting
 * (Settings → Models). `null`/`undefined` means "not customised" → every preset.
 * Otherwise the stored keys, always normalized via [`normalizeVisibleModelKeys`].
 */
export function visibleModelKeys(keys: readonly string[] | null | undefined): string[] {
  return keys == null ? ALL_MODEL_KEYS : normalizeVisibleModelKeys(keys);
}

/**
 * The models the composer's picker should offer. A key matching a preset
 * resolves to it; any other key is a custom model id the user added and resolves
 * via [`modelOption`].
 */
export function visibleModels(keys: readonly string[] | null | undefined): ModelOption[] {
  return visibleModelKeys(keys).map(modelOption);
}

/** A reasoning-effort level. For the standard levels the `key` IS the `--effort`
 *  value the CLI accepts (low | medium | high | xhigh | max). `ultracode` is an
 *  app preset the driver translates to `--effort xhigh` plus the CLI's
 *  `ultracode` settings key — xhigh effort with standing dynamic-workflow
 *  orchestration. It needs an xhigh-capable model (Opus 4.7+, Sonnet 5, Fable 5;
 *  not Sonnet 4.6 / Opus 4.6 / Haiku), else the CLI rejects the session. */
export type EffortOption = { key: string; label: string };

export const EFFORTS: EffortOption[] = [
  { key: "low", label: "Low" },
  { key: "medium", label: "Medium" },
  { key: "high", label: "High" },
  { key: "xhigh", label: "Extra high" },
  { key: "max", label: "Max" },
  { key: "ultracode", label: "Ultracode" },
];

export const DEFAULT_EFFORT_KEY = "high";

export function effortOption(key: string | undefined): EffortOption {
  return EFFORTS.find((e) => e.key === key) ?? EFFORTS[2];
}

// --- ultracode capability guard --------------------------------------------

/** Preset keys for models that support the `xhigh` effort tier — and therefore
 *  the `ultracode` preset (xhigh + workflows). The CLI rejects ultracode on any
 *  other model with "switch to an xhigh-capable model (Fable 5, Opus 4.7+,
 *  Sonnet 5)". Custom/unknown model ids resolve to a key not in this set and are
 *  treated as unsupported. Keep in sync with the CLI's xhigh model list when
 *  adding models. */
const ULTRACODE_MODEL_KEYS = new Set<string>([
  "opus", // Opus 4.8
  "sonnet", // Sonnet 5
  "sonnet-1m", // Sonnet 5 (1M)
  "fable", // Fable 5
  "opus-4-7", // Opus 4.7
  "opus-4-7-1m", // Opus 4.7 (1M)
]);

/** Whether a model can run the `ultracode` effort preset (it needs `xhigh`). */
export function modelSupportsUltracode(model: ModelOption): boolean {
  return ULTRACODE_MODEL_KEYS.has(model.key);
}

/** Resolve the effective effort for a model. `ultracode` needs an xhigh-capable
 *  model; on any other model it downgrades to `high` (the CLI's xhigh gate is the
 *  same, so `high` is the safe floor) so the session isn't rejected. The stored
 *  preference is left untouched, so ultracode re-activates if the user switches
 *  back to a compatible model. */
export function effectiveEffort(key: string | undefined, model: ModelOption): EffortOption {
  const effort = effortOption(key);
  if (effort.key === "ultracode" && !modelSupportsUltracode(model)) {
    return effortOption("high");
  }
  return effort;
}

// --- Codex reasoning effort -------------------------------------------------

/** Display labels for Codex's reasoning-effort values. Codex reports the values
 *  themselves per model (`codexModelList` → `supportedEfforts`) — this only names
 *  them, so a tier we haven't seen still renders (as its raw key) instead of
 *  vanishing from the picker. */
const CODEX_EFFORT_LABELS: Record<string, string> = {
  minimal: "Minimal",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra high",
  max: "Max",
  ultra: "Ultra",
};

/** The effort tiers every Codex model supports, used when the live per-model list
 *  isn't at hand. Deliberately the common core: models differ above `xhigh`
 *  (GPT-5.6-Sol has `ultra`, GPT-5.6-Luna stops at `max`), and offering a tier the
 *  chosen model rejects is worse than offering a short list. Note this is NOT
 *  Claude's ladder — Codex has no `ultracode`. */
export const CODEX_EFFORTS: EffortOption[] = ["low", "medium", "high", "xhigh"].map(
  codexEffortOption,
);

export const DEFAULT_CODEX_EFFORT_KEY = "medium";

/** Label a raw Codex effort value. */
export function codexEffortOption(key: string): EffortOption {
  return { key, label: CODEX_EFFORT_LABELS[key] ?? key };
}

/** The effort choices to offer for a Codex model: exactly what that model
 *  reports, else the common-core fallback. */
export function codexEfforts(supported: readonly string[] | undefined): EffortOption[] {
  return supported?.length ? supported.map(codexEffortOption) : CODEX_EFFORTS;
}

/** Resolve a stored Codex effort key against the tiers actually on offer. An
 *  unset value — or one carried over from another harness (`ultracode`) or from a
 *  model that supported a tier this one doesn't — falls back to `medium`, which
 *  every Codex model has. */
export function resolveCodexEffort(
  key: string | undefined | null,
  supported: readonly string[] | undefined,
): EffortOption {
  const options = codexEfforts(supported);
  return (
    options.find((e) => e.key === key) ??
    options.find((e) => e.key === DEFAULT_CODEX_EFFORT_KEY) ??
    options[0]
  );
}

/**
 * The Codex model ids to offer, given the user's `selectedModels` setting and the
 * live probe. An explicit selection wins; an empty one means "not customised" and
 * falls back to every model Codex reports — the dynamic-list equivalent of how
 * Claude shows all its presets until you narrow them (see [`visibleModelKeys`]).
 * Signed out (empty probe) it yields nothing, which is correct: there's nothing to
 * offer. `probed` is the probe's model list (`CodexProbe.models`).
 */
export function resolveCodexModelIds(
  selected: readonly string[] | null | undefined,
  probed: readonly { id: string }[],
): string[] {
  return selected && selected.length > 0 ? [...selected] : probed.map((m) => m.id);
}
