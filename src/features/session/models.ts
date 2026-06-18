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
  // Latest aliases (the default, Opus, leaves --model unset).
  { key: "opus", label: "Opus 4.8", hint: "1M", flag: null, contextWindow: 1_000_000 },
  { key: "sonnet", label: "Sonnet 4.6", flag: "sonnet", contextWindow: 200_000 },
  { key: "sonnet-1m", label: "Sonnet 4.6", hint: "1M", flag: "sonnet[1m]", contextWindow: 1_000_000 },
  { key: "haiku", label: "Haiku 4.5", flag: "haiku", contextWindow: 200_000 },
  { key: "fable", label: "Fable 5", flag: "fable", contextWindow: 200_000 },
  // Previous Opus versions (full ids — these have no short alias).
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

/** The full list of preset model keys, in display order. */
export const ALL_MODEL_KEYS = MODELS.map((m) => m.key);

/** Set form of [`ALL_MODEL_KEYS`] for O(1) "is this a preset?" membership tests. */
export const ALL_MODEL_KEY_SET = new Set(ALL_MODEL_KEYS);

/**
 * The models the composer's picker should offer, given the user's "visible
 * models" setting (Settings → Models). `null`/`undefined` means "not
 * customised" → show every preset. Otherwise show exactly the listed keys, in
 * the stored order: a key matching a preset resolves to it; any other key is a
 * custom model id the user added and resolves via [`modelOption`].
 */
export function visibleModels(keys: readonly string[] | null | undefined): ModelOption[] {
  if (!keys) return MODELS;
  return keys.map(modelOption);
}

/** A reasoning-effort level. The `key` IS the `--effort` value the CLI accepts
 *  (low | medium | high | xhigh | max). */
export type EffortOption = { key: string; label: string };

export const EFFORTS: EffortOption[] = [
  { key: "low", label: "Low" },
  { key: "medium", label: "Medium" },
  { key: "high", label: "High" },
  { key: "xhigh", label: "Extra high" },
  { key: "max", label: "Max" },
];

export const DEFAULT_EFFORT_KEY = "high";

export function effortOption(key: string | undefined): EffortOption {
  return EFFORTS.find((e) => e.key === key) ?? EFFORTS[2];
}
