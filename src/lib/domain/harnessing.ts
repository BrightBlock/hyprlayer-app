import type { AppSettings, CatalogProvider, Harness, PermissionMode } from "@/lib/bindings";
import {
  MODELS,
  effectiveEffort,
  modelOption,
  resolveCodexEffort,
  type ModelOption,
} from "./models";
import { permissionFlag } from "./permissions";

type HarnessOption = { key: Harness; label: string };

export const HARNESSES: HarnessOption[] = [
  { key: "claude", label: "Claude Code" },
  { key: "codex", label: "Codex" },
  { key: "opencode", label: "OpenCode" },
];

export const HARNESS_SHORT_LABEL: Record<Harness, string> = {
  claude: "Claude",
  codex: "Codex",
  opencode: "OpenCode",
};

export const DEFAULT_HARNESS_OPTIONS: { key: string; label: string }[] = [
  { key: "claude", label: "Claude Code" },
  { key: "codex", label: "Codex" },
  { key: "opencode", label: "OpenCode" },
  { key: "ollama", label: "Ollama" },
];

const UNSUPPORTED_OPENCODE_PROVIDER_IDS = new Set(["github-copilot"]);

export function isSupportedOpencodeProvider(provider: string | null | undefined): boolean {
  return !!provider && !UNSUPPORTED_OPENCODE_PROVIDER_IDS.has(provider.trim().toLowerCase());
}

export function isSupportedOpencodeModel(model: string | null | undefined): model is string {
  if (!model) return false;
  const unprefixed = model.replace(/^opencode:/i, "");
  return isSupportedOpencodeProvider(unprefixed.split("/", 1)[0]);
}

export function firstSupportedOpencodeModel(
  ...models: Array<string | null | undefined>
): string | null {
  return models.find(isSupportedOpencodeModel) ?? null;
}

export function harnessOption(key: string | undefined | null): HarnessOption {
  return HARNESSES.find((h) => h.key === key) ?? HARNESSES[0];
}

export function harnessLabel(h: Harness | null | undefined): string {
  return HARNESSES.find((o) => o.key === h)?.label ?? "the agent";
}

export function effectiveHarnessKey(defaultHarness: string | null | undefined): Harness {
  if (defaultHarness === "opencode" || defaultHarness === "ollama") return "opencode";
  if (defaultHarness === "codex") return "codex";
  return "claude";
}

export type ActionAgentDefaults = {
  harness: Harness;
  model: string | null;
  effort: string | null;
};

export function actionAgentDefaults(
  settings: AppSettings | null | undefined,
): ActionAgentDefaults {
  const providers = settings?.providers;
  const dh = providers?.defaultHarness ?? "claude";
  if (dh === "ollama") {
    const ollama = providers?.ollama;
    const first = ollama?.selectedModels?.[0];
    const model =
      ollama?.commitModel ?? ollama?.defaultModel ?? (first ? `ollama/${first}` : null);
    return { harness: "opencode", model: model ?? null, effort: null };
  }
  if (dh === "opencode") {
    const oc = providers?.opencode;
    const model = firstSupportedOpencodeModel(
      oc?.commitModel,
      oc?.defaultModel,
      ...(oc?.selectedModels ?? []),
    );
    return { harness: "opencode", model, effort: null };
  }
  if (dh === "codex") {
    const codex = providers?.codex;
    const model =
      codex?.commitModel ?? codex?.defaultModel ?? codex?.selectedModels?.[0] ?? null;
    return { harness: "codex", model, effort: codex?.commitEffort ?? "medium" };
  }
  const claude = providers?.claude;
  return {
    harness: "claude",
    model: claude?.commitModel ?? "sonnet",
    effort: claude?.commitEffort ?? "medium",
  };
}

function launchableModel(model: ModelOption, catalog: readonly ModelOption[]): ModelOption {
  return model.disabled ? modelOption(undefined, catalog) : model;
}

export function actionLaunchArgs(
  d: ActionAgentDefaults,
  catalog: readonly ModelOption[] = MODELS,
): {
  harness: Harness;
  model: string | null;
  effort: string | null;
} {
  if (d.harness === "claude") {
    const model = launchableModel(modelOption(d.model ?? undefined, catalog), catalog);
    return {
      harness: "claude",
      model: d.model ? model.flag : null,
      effort: effectiveEffort(d.effort ?? undefined, model)?.key ?? null,
    };
  }
  return {
    harness: d.harness,
    model: d.harness === "opencode" ? firstSupportedOpencodeModel(d.model) : d.model,
    effort: d.harness === "codex" ? d.effort : null,
  };
}

type ShellLaunchPicks = {
  harness?: string;
  model?: string;
  effort?: string;
  opencodeModel?: string;
  codexModel?: string;
  codexEffort?: string;
  permissionMode?: string;
};

export function newChatLaunchArgs(
  settings: AppSettings | null | undefined,
  shell: ShellLaunchPicks | null | undefined,
  catalog: readonly ModelOption[] = MODELS,
): {
  harness: Harness;
  model: string | null;
  effort: string | null;
  permissionMode: PermissionMode | null;
} {
  const providers = settings?.providers;
  const claude = providers?.claude;
  const permissionMode = permissionFlag(shell?.permissionMode ?? (claude?.defaultPlanMode ? "plan" : undefined));
  const harnessKey = shell?.harness ?? effectiveHarnessKey(providers?.defaultHarness);
  if (harnessKey === "opencode") {
    const oc = providers?.opencode;
    const ollama = providers?.ollama;
    const first = ollama?.selectedModels?.[0];
    const model = firstSupportedOpencodeModel(
      shell?.opencodeModel,
      oc?.defaultModel,
      ollama?.defaultModel,
      ...(oc?.selectedModels ?? []),
      first ? `ollama/${first}` : null,
    );
    return { harness: "opencode", model, effort: null, permissionMode };
  }
  if (harnessKey === "codex") {
    const codex = providers?.codex;
    const model = shell?.codexModel ?? codex?.defaultModel ?? codex?.selectedModels?.[0] ?? null;
    return {
      harness: "codex",
      model,
      effort: resolveCodexEffort(shell?.codexEffort ?? codex?.defaultEffort, undefined).key,
      permissionMode,
    };
  }
  const model = launchableModel(
    modelOption(shell?.model ?? claude?.defaultModel ?? undefined, catalog),
    catalog,
  );
  const effort = effectiveEffort(shell?.effort ?? claude?.defaultEffort ?? undefined, model);
  return { harness: "claude", model: model.flag, effort: effort?.key ?? null, permissionMode };
}

export function isFreeOpencodeModel(modelId: string): boolean {
  return modelId.toLowerCase().endsWith("-free");
}

type OpencodeModelName = { model: string; provider: string };

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

export function opencodeModelLabel(
  id: string,
  names: Map<string, OpencodeModelName>,
): string {
  return names.get(id)?.model ?? id.split("/").pop() ?? id;
}

export function modelProvider(
  harnessKey: string,
  opencodeModelId: string | null | undefined,
): string | null {
  if (harnessKey === "opencode") return opencodeModelId ? opencodeModelId.split("/")[0] || null : null;
  if (harnessKey === "codex") return "codex";
  return "claude";
}

export function lockedSessionProvider(
  started: boolean,
  harnessKey: string,
  opencodeModelId: string | null | undefined,
): string | null {
  return started ? modelProvider(harnessKey, opencodeModelId) : null;
}
