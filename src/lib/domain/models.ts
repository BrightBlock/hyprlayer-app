/**
 * The model and reasoning-effort choices offered in the composer. Both map to
 * `claude` CLI flags passed at spawn time (`--model`, `--effort`) — see the
 * driver's `SpawnConfig` (crates/hyprlayer-core/src/claude/driver.rs).
 *
 * Selections are fixed when the session spawns; changing them only affects the
 * next session you start.
 */

import type { ClaudeModel } from "@/lib/bindings";

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
   *  `claude-opus-5` until context crosses 200k, then `[1m]`, which would make
   *  the window flip mid-session). */
  contextWindow: number;
  /** The CLI's own one-liner, when this row came from the live catalog. */
  description?: string;
  /** Effort tiers the CLI says this model accepts. `undefined` means "unknown"
   *  (a static row); an EMPTY array means the model takes no `--effort` at all
   *  (Haiku) — the two are deliberately different. */
  efforts?: string[];
  /** True when the CLI didn't list this model and we're offering it anyway. */
  unlisted?: boolean;
  /** Canonical id a live row resolves to — what resuming onto the null-flag
   *  default row must pass as `--model`. */
  resolvedModel?: string | null;
  /** Listed by the CLI but org-policy restricted. Kept in the catalog so stored
   *  keys resolve to THIS greyed row instead of a launchable static fallback. */
  disabled?: boolean;
};

/**
 * Current models, used ONLY when the live catalog is unreachable (no CLI, not
 * signed in, probe failed). The CLI is the authority — see [`claudeModels`].
 */
const FALLBACK_MODELS: ModelOption[] = [
  // Latest aliases (the default, Opus, leaves --model unset). The `opus` and
  // `sonnet` aliases resolve to Opus 5 / Sonnet 5 in the CLI, so only their
  // labels change as those aliases move.
  { key: "opus", label: "Opus 5", hint: "1M", flag: null, contextWindow: 1_000_000 },
  { key: "sonnet", label: "Sonnet 5", flag: "sonnet", contextWindow: 200_000 },
  { key: "sonnet-1m", label: "Sonnet 5", hint: "1M", flag: "sonnet[1m]", contextWindow: 1_000_000 },
  { key: "haiku", label: "Haiku 4.5", flag: "haiku", contextWindow: 200_000 },
  { key: "fable", label: "Fable 5", hint: "1M", flag: "fable", contextWindow: 1_000_000 },
];

/**
 * Older models the CLI's catalog no longer lists but `--model` still accepts —
 * it validates the flag lazily, so an unlisted id fails at the first API call
 * rather than at spawn. Kept so a pin doesn't vanish when Anthropic retires a
 * row from the picker, and always shown; the live catalog can't confirm them,
 * so they render as unlisted.
 */
const LEGACY_MODELS: ModelOption[] = [
  { key: "sonnet-4-6", label: "Sonnet 4.6", flag: "claude-sonnet-4-6", contextWindow: 200_000 },
  { key: "sonnet-4-6-1m", label: "Sonnet 4.6", hint: "1M", flag: "claude-sonnet-4-6[1m]", contextWindow: 1_000_000 },
  { key: "opus-4-8", label: "Opus 4.8", flag: "claude-opus-4-8", contextWindow: 200_000 },
  { key: "opus-4-8-1m", label: "Opus 4.8", hint: "1M", flag: "claude-opus-4-8[1m]", contextWindow: 1_000_000 },
  { key: "opus-4-7", label: "Opus 4.7", flag: "claude-opus-4-7", contextWindow: 200_000 },
  { key: "opus-4-7-1m", label: "Opus 4.7", hint: "1M", flag: "claude-opus-4-7[1m]", contextWindow: 1_000_000 },
  { key: "opus-4-6", label: "Opus 4.6", flag: "claude-opus-4-6", contextWindow: 200_000 },
  { key: "opus-4-6-1m", label: "Opus 4.6", hint: "1M", flag: "claude-opus-4-6[1m]", contextWindow: 1_000_000 },
  { key: "opus-4-1", label: "Opus 4.1", flag: "claude-opus-4-1", contextWindow: 200_000 },
];

/** Every static preset, in display order — the catalog-less list, and the key
 *  space [`modelOption`] resolves against. */
export const MODELS: ModelOption[] = [...FALLBACK_MODELS, ...LEGACY_MODELS];

/** A `[1m]`-suffixed flag or resolved id is the CLI's 1M-context variant. */
function isOneMillion(...ids: (string | null | undefined)[]): boolean {
  return ids.some((id) => id?.includes("[1m]"));
}

/**
 * A versioned display name for a resolved model id: `claude-opus-4-6[1m]` →
 * `Opus 4.6`, `claude-haiku-4-5-20251001` → `Haiku 4.5`.
 *
 * The CLI's own labels are deliberately vague ("Default (recommended)", "Opus",
 * "Sonnet") because its picker only ever shows one of each family. Ours shows
 * current AND older models together, so the version has to be on the row —
 * otherwise "Opus" and "Opus 4.6" sit next to each other meaning different
 * things. Returns `undefined` for anything that isn't a `claude-<family>-<ver>`
 * id, so callers fall back to whatever label they already had.
 */
export function modelDisplayName(resolvedId: string | null | undefined): string | undefined {
  if (!resolvedId) return undefined;
  const bare = resolvedId
    .replace(/\[[^\]]*\]$/, "") // variant suffix: [1m]
    .replace(/^claude-/, "")
    .replace(/-\d{8}(-v\d+)?$/, ""); // dated snapshot: -20251001, -20250805-v1
  const [family, ...version] = bare.split("-");
  // Every current family is a single word; a leftover non-numeric segment means
  // an id shape we don't understand, so don't guess at a name for it.
  if (!family || !/^[a-z]+$/.test(family) || !version.every((v) => /^\d+$/.test(v))) {
    return undefined;
  }
  const name = family[0].toUpperCase() + family.slice(1);
  return version.length > 0 ? `${name} ${version.join(".")}` : name;
}

/** The preset a live row's LAUNCH FLAG re-states — the unambiguous identity.
 *  The null-flag row is the CLI's "default" and owns the default key. */
function presetKeyByFlag(m: ClaudeModel): string | undefined {
  if (m.flag === null) return DEFAULT_MODEL_KEY;
  return MODELS.find((s) => s.flag !== null && s.flag === m.flag)?.key;
}

/** The preset a live row re-states by resolved name + window — weaker than a
 *  flag match, so it must never rob one. Null-flag presets are excluded: only
 *  the CLI's own "default" row may key as the default. */
function presetKeyByName(m: ClaudeModel, oneMillion: boolean): string | undefined {
  const name = modelDisplayName(m.resolvedModel);
  if (!name) return undefined;
  return MODELS.find(
    (s) => s.flag !== null && s.label === name && (s.contextWindow === 1_000_000) === oneMillion,
  )?.key;
}

/**
 * The CLI's own catalog (`list_models`), mapped into picker rows. A row that
 * re-states one of our static presets takes that preset's KEY (so stored
 * settings — `"fable"`, `"sonnet-1m"` — keep resolving, and the model sync
 * doesn't re-offer a model under a new name); a genuinely new model keys off
 * the flag it launches with. Org-disabled rows stay, marked, so stored keys
 * resolve to the greyed row rather than a launchable static fallback.
 *
 * Keys are claimed in two passes so the result is order-independent: flag
 * identity first (an alias row always keeps its own alias), then name+window,
 * then the raw flag. A row whose every candidate is already claimed is a
 * duplicate spelling of a model on offer and is dropped — stored keys resolve
 * by key, so two rows must never share one.
 */
export function liveModelOptions(models: readonly ClaudeModel[]): ModelOption[] {
  const keys: (string | undefined)[] = models.map(() => undefined);
  const taken = new Set<string>();
  models.forEach((m, i) => {
    const k = presetKeyByFlag(m);
    if (k && !taken.has(k)) {
      keys[i] = k;
      taken.add(k);
    }
  });
  models.forEach((m, i) => {
    if (keys[i]) return;
    const byName = presetKeyByName(m, isOneMillion(m.flag, m.resolvedModel));
    // Last resort before dropping: the resolved id. Distinguishes a real row
    // whose flag another row claimed (a literal `opus` alias when the default
    // resolves elsewhere) from a duplicate spelling of the same launch.
    const candidates = [byName, m.flag, m.resolvedModel];
    const k = candidates.find((c): c is string => Boolean(c) && !taken.has(c!));
    if (!k) return;
    keys[i] = k;
    taken.add(k);
  });
  return models.flatMap((m, i) => {
    const key = keys[i];
    if (!key) return [];
    const oneMillion = isOneMillion(m.flag, m.resolvedModel);
    // The row that launches with no `--model` follows the user's CLI config,
    // so say which model that is AND that it's the default — both rows can
    // otherwise resolve to the same name.
    const hint = [oneMillion ? "1M" : null, m.flag === null ? "default" : null]
      .filter(Boolean)
      .join(" · ");
    return [
      {
        key,
        // Prefer the versioned name over the CLI's vaguer label ("Opus",
        // "Default (recommended)") — see `modelDisplayName`.
        label: modelDisplayName(m.resolvedModel) ?? m.displayName,
        hint: hint || undefined,
        flag: m.flag,
        contextWindow: oneMillion ? 1_000_000 : 200_000,
        description: m.description ?? undefined,
        efforts: m.supportedEffortLevels,
        resolvedModel: m.resolvedModel,
        ...(m.disabled ? { disabled: true } : {}),
      },
    ];
  });
}

/**
 * The models to offer: the CLI's live catalog when we have one, else the static
 * current-model rows — plus the legacy pins the catalog never lists, minus any
 * the catalog turns out to cover after all.
 */
export function claudeModels(live: readonly ClaudeModel[] | null | undefined): ModelOption[] {
  const asked = Boolean(live?.length);
  const current = asked ? liveModelOptions(live!) : FALLBACK_MODELS;
  const listed = new Set(current.map((m) => m.flag));
  // A live row can cover a legacy pin under a different flag spelling (its
  // canonical key match) — dedupe by key as well so no key appears twice.
  const listedKeys = new Set(current.map((m) => m.key));
  return [
    ...current,
    // Only a catalog we actually got back can call a row unlisted; without one
    // we never asked, so the legacy rows carry no claim either way.
    ...LEGACY_MODELS.filter((m) => !listed.has(m.flag) && !listedKeys.has(m.key)).map((m) =>
      asked ? { ...m, unlisted: true } : m,
    ),
  ];
}

export const DEFAULT_MODEL_KEY = "opus";

/** Resolve a stored model key to its option, against the live catalog when the
 *  caller has one and the static presets otherwise. An empty/undefined key falls
 *  back to the catalog's first row (the default). Any other non-empty value is
 *  treated as a CUSTOM model the user typed — an alias (`haiku`) or full id
 *  (`claude-opus-4-7`) the CLI's `--model` accepts. For a custom value the string
 *  IS both the flag and its own label, since we can't know a friendly name for an
 *  arbitrary id. */
export function modelOption(
  key: string | undefined,
  catalog: readonly ModelOption[] = MODELS,
): ModelOption {
  // No stored pick ⇒ the catalog's default row — skipping an org-disabled one,
  // which must never become the implicit active selection.
  if (!key) return catalog.find((m) => !m.disabled) ?? catalog[0] ?? MODELS[0];
  // Fall through to the static presets so a key the live catalog dropped (a
  // legacy pin, or a row this account no longer sees) still resolves to its real
  // flag instead of being mistaken for a custom id.
  const preset = catalog.find((m) => m.key === key) ?? MODELS.find((m) => m.key === key);
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
 *  `claude-opus-5[1m]`), or `undefined` when the fetched catalog doesn't cover
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
 *  omitted, so switching TO the default entry (whose launch `flag` is null)
 *  mid-conversation needs an explicit id — the live row's own `resolvedModel`
 *  when the probe gave us one, else the static assumption (Opus 5). */
export function resumeModelFlag(model: ModelOption): string {
  return model.flag ?? model.resolvedModel ?? "claude-opus-5";
}

/** The full list of static preset model keys, in display order. */
export const ALL_MODEL_KEYS = MODELS.map((m) => m.key);

/**
 * The preset keys the picker offered before it read the CLI's catalog — a
 * FROZEN historical snapshot, not a live list. It seeds the "already seen"
 * watermark for installs that predate [`ClaudeSettings.seenModelKeys`], so the
 * first sync can tell a model the user removed on purpose (in here) from one
 * that simply didn't exist yet (not in here). Never add to it: a new model
 * belongs in [`MODELS`], and adding it here would stop that model reaching
 * anyone with a curated list.
 */
const PRE_CATALOG_MODEL_KEYS = [
  "opus",
  "sonnet",
  "sonnet-1m",
  "haiku",
  "fable",
  "sonnet-4-6",
  "sonnet-4-6-1m",
  "opus-4-7",
  "opus-4-7-1m",
  "opus-4-6",
  "opus-4-6-1m",
  "opus-4-1",
];

/** What [`claudeSyncModels`] should write, or `null` when nothing changed. */
export type ModelSync = { add: string[]; seen: string[] };

/**
 * Work out which models to fold into a curated picker list, given what the
 * catalog offers now and what it offered last time.
 *
 * `add` is what the user has never been offered — new to them, so it joins their
 * list. Anything they once saw and dropped stays dropped. An uncurated list
 * (`visibleModels == null`) already shows everything, so `add` is empty and only
 * the watermark moves. Returns `null` when the watermark is already current, so
 * the caller can skip the write entirely.
 */
export function pendingModelSync(
  visibleModels: readonly string[] | null | undefined,
  seenModelKeys: readonly string[] | null | undefined,
  catalog: readonly ModelOption[],
): ModelSync | null {
  const seen = new Set(seenModelKeys ?? PRE_CATALOG_MODEL_KEYS);
  // Org-disabled models are neither offered nor marked seen — if policy later
  // relents, the model is still "new" and reaches the user then.
  const disabled = new Set(catalog.filter((m) => m.disabled).map((m) => m.key));
  const offered = knownModelKeys(catalog).filter((key) => !disabled.has(key));
  const fresh = offered.filter((key) => !seen.has(key));
  if (fresh.length === 0) return null;
  // Curated lists grow; an uncurated one needs no `add` (it shows everything),
  // but its watermark must still advance or we'd re-offer on every launch.
  return { add: visibleModels == null ? [] : fresh, seen: [...seen, ...fresh] };
}

/** Set form of [`ALL_MODEL_KEYS`] for O(1) "is this a preset?" membership tests. */
export const ALL_MODEL_KEY_SET = new Set(ALL_MODEL_KEYS);

/**
 * Every key we can name a model for, in display order: the catalog first, then
 * any static preset it doesn't carry. That tail matters — a live catalog can
 * omit a variant we still know (this account isn't offered `sonnet[1m]`, say),
 * and those are presets [`modelOption`] resolves, not user-typed custom ids.
 */
export function knownModelKeys(catalog: readonly ModelOption[] = MODELS): string[] {
  const inCatalog = new Set(catalog.map((m) => m.key));
  return [...inCatalog, ...ALL_MODEL_KEYS.filter((key) => !inCatalog.has(key))];
}

/**
 * Canonical ordering for a "visible models" list: known keys in display order,
 * then custom ids in first-seen order, de-duplicated and emptied of falsy keys.
 * The single source of truth for normalization — both the Settings UI (on write)
 * and the composer (on read) run through it, so a stored list that predates this
 * rule still renders tidily.
 */
export function normalizeVisibleModelKeys(
  keys: readonly string[],
  catalog: readonly ModelOption[] = MODELS,
): string[] {
  // `[...seen]` preserves first-occurrence order, so customs keep insertion order.
  const seen = new Set(keys.filter(Boolean));
  const known = knownModelKeys(catalog);
  const isKnown = new Set(known);
  return [...known.filter((key) => seen.has(key)), ...[...seen].filter((key) => !isKnown.has(key))];
}

/**
 * The visible model keys to offer, given the user's "visible models" setting
 * (Settings → Models). `null`/`undefined` means "not customised" → the whole
 * catalog. Otherwise the stored keys, always normalized via
 * [`normalizeVisibleModelKeys`].
 */
export function visibleModelKeys(
  keys: readonly string[] | null | undefined,
  catalog: readonly ModelOption[] = MODELS,
): string[] {
  // Uncurated shows the catalog only — the static tail from `knownModelKeys` is
  // for resolving what someone already picked, not for padding the default list.
  // Org-disabled rows are offered to nobody; a curated list that already names
  // one keeps it (it resolves to the greyed row, which says why it's dead).
  return keys == null
    ? catalog.filter((m) => !m.disabled).map((m) => m.key)
    : normalizeVisibleModelKeys(keys, catalog);
}

/**
 * The models the composer's picker should offer. A key matching a preset
 * resolves to it; any other key is a custom model id the user added and resolves
 * via [`modelOption`].
 */
export function visibleModels(
  keys: readonly string[] | null | undefined,
  catalog: readonly ModelOption[] = MODELS,
): ModelOption[] {
  return visibleModelKeys(keys, catalog).map((key) => modelOption(key, catalog));
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

/** Fallback for models with no live capability data: preset keys that support
 *  the `xhigh` effort tier, and therefore the `ultracode` preset. The CLI rejects
 *  ultracode elsewhere with "switch to an xhigh-capable model (Fable 5, Opus
 *  4.7+, Sonnet 5)". Only consulted for static rows — a live row answers from its
 *  own [`ModelOption.efforts`]. */
const ULTRACODE_MODEL_KEYS = new Set<string>([
  "opus", // Opus 5
  "sonnet", // Sonnet 5
  "sonnet-1m", // Sonnet 5 (1M)
  "fable", // Fable 5
  "opus-4-8", // Opus 4.8
  "opus-4-8-1m", // Opus 4.8 (1M)
  "opus-4-7", // Opus 4.7
  "opus-4-7-1m", // Opus 4.7 (1M)
]);

/** Whether a model can run the `ultracode` effort preset (it needs `xhigh`). */
export function modelSupportsUltracode(model: ModelOption): boolean {
  return model.efforts ? model.efforts.includes("xhigh") : ULTRACODE_MODEL_KEYS.has(model.key);
}

/** The effort tiers to offer for a model. A live row reports its own — an empty
 *  list means the model takes no `--effort` at all (Haiku), so the picker should
 *  be hidden rather than filled with tiers the CLI ignores. Static rows fall back
 *  to the full ladder, with `ultracode` gated as before. */
export function effortsForModel(model: ModelOption): EffortOption[] {
  if (!model.efforts) return EFFORTS;
  if (model.efforts.length === 0) return [];
  // Label a tier we don't know as itself rather than letting `effortOption`'s
  // default silently render it as "High".
  const tiers = model.efforts.map((key) => EFFORTS.find((e) => e.key === key) ?? { key, label: key });
  return model.efforts.includes("xhigh") ? [...tiers, effortOption("ultracode")] : tiers;
}

/** Resolve the effective effort for a model — what actually rides `--effort`,
 *  or `null` to omit the flag (ONLY for a model that takes no effort at all).
 *  `ultracode` needs an xhigh-capable model and downgrades to `high` elsewhere;
 *  a live row's own tier list then clamps the result, so a stored tier the CLI
 *  would reject (`max` on a capped model) never reaches a spawn. The clamp
 *  always names an explicit SUPPORTED tier — `high` when the model has it, else
 *  its top tier — because an omitted `--effort` on `--resume` inherits the old
 *  session's effort rather than resetting it (see `restart_session`). The
 *  stored preference is left untouched, so it re-activates on a model that
 *  supports it. */
export function effectiveEffort(key: string | undefined, model: ModelOption): EffortOption | null {
  if (model.efforts?.length === 0) return null; // takes no --effort at all
  // A stored tier the model itself reports is the user's pick even when the
  // static ladder can't name it — `effortOption` would silently turn a new
  // CLI tier ("ultra") into High.
  if (key && model.efforts?.includes(key)) {
    return EFFORTS.find((e) => e.key === key) ?? { key, label: key };
  }
  let effort = effortOption(key);
  if (effort.key === "ultracode") {
    if (modelSupportsUltracode(model)) return effort;
    effort = effortOption("high");
  }
  if (model.efforts && !model.efforts.includes(effort.key)) {
    if (model.efforts.includes(DEFAULT_EFFORT_KEY)) return effortOption(DEFAULT_EFFORT_KEY);
    const ladder = EFFORTS.filter((e) => model.efforts!.includes(e.key));
    const top = ladder[ladder.length - 1]?.key ?? model.efforts[model.efforts.length - 1];
    return EFFORTS.find((e) => e.key === top) ?? { key: top, label: top };
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
