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

export const MODELS: ModelOption[] = [
  { key: "opus", label: "Opus 4.8", hint: "1M", flag: null, contextWindow: 1_000_000 },
  { key: "sonnet", label: "Sonnet 4.6", flag: "sonnet", contextWindow: 200_000 },
  { key: "haiku", label: "Haiku 4.5", flag: "haiku", contextWindow: 200_000 },
  { key: "fable", label: "Fable 5", flag: "fable", contextWindow: 200_000 },
];

export const DEFAULT_MODEL_KEY = "opus";

export function modelOption(key: string | undefined): ModelOption {
  return MODELS.find((m) => m.key === key) ?? MODELS[0];
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
