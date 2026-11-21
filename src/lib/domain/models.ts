
import type { ClaudeModel } from "@/lib/bindings";
import type { DropdownOption } from "@/components/ui";

export type ModelOption = {
  key: string;
  label: string;
  hint?: string;
  flag: string | null;
  description?: string;
  efforts?: string[];
  unlisted?: boolean;
  resolvedModel?: string | null;
  disabled?: boolean;
};

const FALLBACK_MODELS: ModelOption[] = [
  { key: "opus", label: "Opus 5", hint: "1M", flag: null },
  { key: "sonnet", label: "Sonnet 5", flag: "sonnet" },
  { key: "sonnet-1m", label: "Sonnet 5", hint: "1M", flag: "sonnet[1m]" },
  { key: "haiku", label: "Haiku 4.5", flag: "haiku" },
  { key: "fable", label: "Fable 5", hint: "1M", flag: "fable" },
];

const LEGACY_MODELS: ModelOption[] = [
  { key: "sonnet-4-6", label: "Sonnet 4.6", flag: "claude-sonnet-4-6" },
  { key: "sonnet-4-6-1m", label: "Sonnet 4.6", hint: "1M", flag: "claude-sonnet-4-6[1m]" },
  { key: "opus-4-8", label: "Opus 4.8", flag: "claude-opus-4-8" },
  { key: "opus-4-8-1m", label: "Opus 4.8", hint: "1M", flag: "claude-opus-4-8[1m]" },
  { key: "opus-4-7", label: "Opus 4.7", flag: "claude-opus-4-7" },
  { key: "opus-4-7-1m", label: "Opus 4.7", hint: "1M", flag: "claude-opus-4-7[1m]" },
  { key: "opus-4-6", label: "Opus 4.6", flag: "claude-opus-4-6" },
  { key: "opus-4-6-1m", label: "Opus 4.6", hint: "1M", flag: "claude-opus-4-6[1m]" },
  { key: "opus-4-1", label: "Opus 4.1", flag: "claude-opus-4-1" },
];

export const MODELS: ModelOption[] = [...FALLBACK_MODELS, ...LEGACY_MODELS];

function isOneMillion(...ids: (string | null | undefined)[]): boolean {
  return ids.some((id) => id?.includes("[1m]"));
}

export function modelDisplayName(resolvedId: string | null | undefined): string | undefined {
  if (!resolvedId) return undefined;
  const bare = resolvedId
    .replace(/\[[^\]]*\]$/, "")
    .replace(/^claude-/, "")
    .replace(/-\d{8}(-v\d+)?$/, "");
  const [family, ...version] = bare.split("-");
  if (!family || !/^[a-z]+$/.test(family) || !version.every((v) => /^\d+$/.test(v))) {
    return undefined;
  }
  const name = family[0].toUpperCase() + family.slice(1);
  return version.length > 0 ? `${name} ${version.join(".")}` : name;
}

function presetKeyByFlag(m: ClaudeModel): string | undefined {
  if (m.flag === null) return DEFAULT_MODEL_KEY;
  return MODELS.find((s) => s.flag !== null && s.flag === m.flag)?.key;
}

function presetKeyByName(m: ClaudeModel, oneMillion: boolean): string | undefined {
  const name = modelDisplayName(m.resolvedModel);
  if (!name) return undefined;
  return MODELS.find(
    (s) => s.flag !== null && s.label === name && (s.hint === "1M") === oneMillion,
  )?.key;
}

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
    const isDefault = m.flag === null;
    const name = modelDisplayName(m.resolvedModel) ?? m.displayName.replace(/\s*\(disabled\)$/i, "");
    const hint = [isDefault ? name : null, oneMillion ? "1M" : null].filter(Boolean).join(" · ");
    return [
      {
        key,
        label: isDefault ? "Default" : name,
        hint: hint || undefined,
        flag: m.flag,
        description: m.description ?? undefined,
        efforts: m.supportedEffortLevels,
        resolvedModel: m.resolvedModel,
        ...(m.disabled ? { disabled: true } : {}),
      },
    ];
  });
}

export function claudeModels(live: readonly ClaudeModel[] | null | undefined): ModelOption[] {
  const asked = Boolean(live?.length);
  const current = asked ? liveModelOptions(live!) : FALLBACK_MODELS;
  const listed = new Set(current.map((m) => m.flag));
  const listedKeys = new Set(current.map((m) => m.key));
  return [
    ...current,
    ...LEGACY_MODELS.filter((m) => !listed.has(m.flag) && !listedKeys.has(m.key)).map((m) =>
      asked ? { ...m, unlisted: true } : m,
    ),
  ];
}

export const DEFAULT_MODEL_KEY = "opus";

export function modelOption(
  key: string | undefined,
  catalog: readonly ModelOption[] = MODELS,
): ModelOption {
  if (!key) return catalog.find((m) => !m.disabled) ?? catalog[0] ?? MODELS[0];
  const preset = catalog.find((m) => m.key === key) ?? MODELS.find((m) => m.key === key);
  if (preset) return preset;
  return {
    key,
    label: key,
    flag: key,
  };
}

// Init may report an id without the date or `[1m]` the catalog's carries, or with them.
const bareModelId = (id: string) => id.replace(/\[[^\]]*\]$/, "").replace(/-\d{8}(-v\d+)?$/, "");

/** The catalog key for a launch's model flag or id; an unknown one stays as given. */
export function claudeModelKey(flagOrId: string, catalog: readonly ModelOption[]): string {
  const exact = catalog.find((m) => m.key === flagOrId || m.flag === flagOrId || m.resolvedModel === flagOrId);
  if (exact) return exact.key;
  const bare = bareModelId(flagOrId);
  return catalog.find((m) => m.resolvedModel && bareModelId(m.resolvedModel) === bare)?.key ?? flagOrId;
}

export function resumeModelFlag(model: ModelOption): string {
  return model.flag ?? model.resolvedModel ?? "claude-opus-5";
}

export const ALL_MODEL_KEYS = MODELS.map((m) => m.key);

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

export type ModelSync = { add: string[]; seen: string[] };

export function pendingModelSync(
  visibleModels: readonly string[] | null | undefined,
  seenModelKeys: readonly string[] | null | undefined,
  catalog: readonly ModelOption[],
): ModelSync | null {
  const seen = new Set(seenModelKeys ?? PRE_CATALOG_MODEL_KEYS);
  const disabled = new Set(catalog.filter((m) => m.disabled).map((m) => m.key));
  const offered = knownModelKeys(catalog).filter((key) => !disabled.has(key));
  const fresh = offered.filter((key) => !seen.has(key));
  if (fresh.length === 0) return null;
  return { add: visibleModels == null ? [] : fresh, seen: [...seen, ...fresh] };
}

export function knownModelKeys(catalog: readonly ModelOption[] = MODELS): string[] {
  const inCatalog = new Set(catalog.map((m) => m.key));
  return [...inCatalog, ...ALL_MODEL_KEYS.filter((key) => !inCatalog.has(key))];
}

export function normalizeVisibleModelKeys(
  keys: readonly string[],
  catalog: readonly ModelOption[] = MODELS,
): string[] {
  const seen = new Set(keys.filter(Boolean));
  const known = knownModelKeys(catalog);
  const isKnown = new Set(known);
  return [...known.filter((key) => seen.has(key)), ...[...seen].filter((key) => !isKnown.has(key))];
}

export function visibleModelKeys(
  keys: readonly string[] | null | undefined,
  catalog: readonly ModelOption[] = MODELS,
): string[] {
  return keys == null
    ? catalog.filter((m) => !m.disabled).map((m) => m.key)
    : normalizeVisibleModelKeys(keys, catalog);
}

export function visibleModels(
  keys: readonly string[] | null | undefined,
  catalog: readonly ModelOption[] = MODELS,
): ModelOption[] {
  const shown = new Set(visibleModelKeys(keys, catalog));
  for (const m of catalog) if (m.disabled) shown.add(m.key);
  return normalizeVisibleModelKeys([...shown], catalog).map((key) => modelOption(key, catalog));
}

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


const ULTRACODE_MODEL_KEYS = new Set<string>([
  "opus",
  "sonnet",
  "sonnet-1m",
  "fable",
  "opus-4-8",
  "opus-4-8-1m",
  "opus-4-7",
  "opus-4-7-1m",
]);

export function modelSupportsUltracode(model: ModelOption): boolean {
  return model.efforts ? model.efforts.includes("xhigh") : ULTRACODE_MODEL_KEYS.has(model.key);
}

export function effortsForModel(model: ModelOption): EffortOption[] {
  if (!model.efforts) return EFFORTS;
  if (model.efforts.length === 0) return [];
  const tiers = model.efforts.map((key) => EFFORTS.find((e) => e.key === key) ?? { key, label: key });
  return model.efforts.includes("xhigh") ? [...tiers, effortOption("ultracode")] : tiers;
}

export function effectiveEffort(key: string | undefined, model: ModelOption): EffortOption | null {
  if (model.efforts?.length === 0) return null;
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


const CODEX_EFFORT_LABELS: Record<string, string> = {
  minimal: "Minimal",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra high",
  max: "Max",
  ultra: "Ultra",
};

export const CODEX_EFFORTS: EffortOption[] = ["low", "medium", "high", "xhigh"].map(
  codexEffortOption,
);

export const DEFAULT_CODEX_EFFORT_KEY = "medium";

export function codexEffortOption(key: string): EffortOption {
  return { key, label: CODEX_EFFORT_LABELS[key] ?? key };
}

export function codexEfforts(supported: readonly string[] | undefined): EffortOption[] {
  return supported?.length ? supported.map(codexEffortOption) : CODEX_EFFORTS;
}

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

export function resolveCodexModelIds(
  selected: readonly string[] | null | undefined,
  probed: readonly { id: string }[],
): string[] {
  return selected && selected.length > 0 ? [...selected] : probed.map((m) => m.id);
}

export function modelOptions(
  catalog: readonly ModelOption[],
  value: (m: ModelOption) => string = (m) => m.key,
): DropdownOption[] {
  return catalog.map((m) => ({
    value: value(m),
    label: m.hint ? `${m.label} (${m.hint})` : m.label,
    ...(m.disabled ? { disabled: true } : {}),
  }));
}
