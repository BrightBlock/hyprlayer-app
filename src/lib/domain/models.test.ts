import { describe, expect, it } from "vitest";

import type { ClaudeModel } from "@/lib/bindings";

import {
  claudeModels,
  CODEX_EFFORTS,
  codexEfforts,
  resolveCodexEffort,
  resolveCodexModelIds,
  EFFORTS,
  effectiveEffort,
  effortsForModel,
  MODELS,
  modelDisplayName,
  modelOption,
  modelSupportsUltracode,
  pendingModelSync,
  resumeModelFlag,
  visibleModelKeys,
} from "./models";

function row(over: Partial<ClaudeModel> = {}): ClaudeModel {
  return {
    flag: null,
    resolvedModel: null,
    displayName: "Model",
    description: null,
    supportedEffortLevels: ["low", "medium", "high", "xhigh", "max"],
    supportsFastMode: false,
    disabled: false,
    ...over,
  };
}

describe("Codex reasoning effort", () => {
  it("offers each model's OWN tiers, not a fixed ladder", () => {
    expect(codexEfforts(["low", "medium", "high", "xhigh", "max", "ultra"]).map((e) => e.key))
      .toEqual(["low", "medium", "high", "xhigh", "max", "ultra"]);
    expect(codexEfforts(["low", "medium", "high", "xhigh", "max"]).map((e) => e.key))
      .toEqual(["low", "medium", "high", "xhigh", "max"]);
  });

  it("falls back to the tiers every Codex model supports", () => {
    const keys = CODEX_EFFORTS.map((e) => e.key);
    expect(keys).toEqual(["low", "medium", "high", "xhigh"]);
    expect(codexEfforts(undefined).map((e) => e.key)).toEqual(keys);
    expect(codexEfforts([]).map((e) => e.key)).toEqual(keys);
  });

  it("labels a tier it hasn't seen instead of dropping it", () => {
    expect(codexEfforts(["low", "hyperdrive"]).map((e) => e.label)).toEqual(["Low", "hyperdrive"]);
  });

  it("resolves a stored key against what the model actually supports", () => {
    const sol = ["low", "medium", "high", "xhigh", "max", "ultra"];
    expect(resolveCodexEffort("ultra", sol).key).toBe("ultra");
    expect(resolveCodexEffort("ultra", ["low", "medium", "high", "xhigh", "max"]).key)
      .toBe("medium");
    expect(resolveCodexEffort(undefined, sol).key).toBe("medium");
    expect(resolveCodexEffort("ultracode", sol).key).toBe("medium");
    expect(resolveCodexEffort(null, undefined).key).toBe("medium");
  });
});

describe("resolveCodexModelIds", () => {
  const probed = [{ id: "gpt-5.4-codex" }, { id: "gpt-5.4" }, { id: "gpt-5.4-mini" }];

  it("offers every probed model when the user hasn't curated a selection", () => {
    expect(resolveCodexModelIds(undefined, probed)).toEqual([
      "gpt-5.4-codex",
      "gpt-5.4",
      "gpt-5.4-mini",
    ]);
    expect(resolveCodexModelIds(null, probed)).toEqual(["gpt-5.4-codex", "gpt-5.4", "gpt-5.4-mini"]);
    expect(resolveCodexModelIds([], probed)).toEqual(["gpt-5.4-codex", "gpt-5.4", "gpt-5.4-mini"]);
  });

  it("honours an explicit selection over the full probe", () => {
    expect(resolveCodexModelIds(["gpt-5.4"], probed)).toEqual(["gpt-5.4"]);
  });

  it("yields nothing when signed out (no models to offer)", () => {
    expect(resolveCodexModelIds(undefined, [])).toEqual([]);
    expect(resolveCodexModelIds([], [])).toEqual([]);
  });
});

describe("Claude model presets", () => {
  it("tracks the CLI's `opus` alias, now Opus 5", () => {
    expect(modelOption("opus")).toMatchObject({ label: "Opus 5", flag: null });
    expect(resumeModelFlag(modelOption("opus"))).toBe("claude-opus-5");
  });

  it("keeps Opus 4.8 pinnable now that the alias has moved off it", () => {
    expect(modelOption("opus-4-8").flag).toBe("claude-opus-4-8");
    expect(modelOption("opus-4-8-1m")).toMatchObject({
      flag: "claude-opus-4-8[1m]",
      contextWindow: 1_000_000,
    });
  });

  it("keys every preset uniquely, stored settings resolve by key", () => {
    expect(new Set(MODELS.map((m) => m.key)).size).toBe(MODELS.length);
  });
});

describe("modelDisplayName", () => {
  it("renders the family and version the CLI's label leaves out", () => {
    expect(modelDisplayName("claude-opus-5")).toBe("Opus 5");
    expect(modelDisplayName("claude-opus-4-6")).toBe("Opus 4.6");
    expect(modelDisplayName("claude-sonnet-4-6")).toBe("Sonnet 4.6");
    expect(modelDisplayName("claude-fable-5")).toBe("Fable 5");
  });

  it("strips the 1M variant suffix and dated snapshots", () => {
    expect(modelDisplayName("claude-opus-5[1m]")).toBe("Opus 5");
    expect(modelDisplayName("claude-haiku-4-5-20251001")).toBe("Haiku 4.5");
    expect(modelDisplayName("claude-opus-4-1-20250805-v1")).toBe("Opus 4.1");
  });

  it("declines to guess at an id shape it doesn't recognise", () => {
    expect(modelDisplayName("anthropic/claude-opus-5")).toBeUndefined();
    expect(modelDisplayName("ollama/qwen2.5-coder")).toBeUndefined();
    expect(modelDisplayName("gpt-5.6-sol")).toBeUndefined();
    expect(modelDisplayName(null)).toBeUndefined();
    expect(modelDisplayName("")).toBeUndefined();
  });
});

describe("live catalog merge", () => {
  const live = [
    row({ flag: null, resolvedModel: "claude-opus-5[1m]", displayName: "Default (recommended)" }),
    row({ flag: "sonnet", resolvedModel: "claude-sonnet-5", displayName: "Sonnet" }),
    row({
      flag: "haiku",
      resolvedModel: "claude-haiku-4-5-20251001",
      displayName: "Haiku",
      supportedEffortLevels: [],
    }),
  ];

  it("falls back to the static table when there is no catalog", () => {
    expect(claudeModels(null)).toEqual(MODELS);
    expect(claudeModels([])).toEqual(MODELS);
  });

  it("prefers live rows and keeps the legacy pins the CLI omits", () => {
    const merged = claudeModels(live);
    expect(merged.slice(0, 3).map((m) => m.label)).toEqual(["Default", "Sonnet 5", "Haiku 4.5"]);
    expect(merged[0].hint).toBe("Opus 5 · 1M");
    const legacy = merged.find((m) => m.key === "opus-4-7");
    expect(legacy).toMatchObject({ flag: "claude-opus-4-7", unlisted: true });
    expect(merged.filter((m) => m.flag === "sonnet")).toHaveLength(1);
  });

  it("keys the null-flag row as the default so stored settings still resolve", () => {
    const merged = claudeModels(live);
    expect(modelOption("opus", merged)).toMatchObject({ flag: null, label: "Default" });
    expect(modelOption("sonnet-1m", merged).flag).toBe("sonnet[1m]");
  });

  it("reads the 1M window off the flag or the resolved id", () => {
    const merged = claudeModels(live);
    expect(merged[0].contextWindow).toBe(1_000_000);
    expect(merged[1].contextWindow).toBe(200_000);
  });

  it("keeps org-disabled models, marked, so stored keys can't resurrect them", () => {
    const merged = claudeModels([...live, row({ flag: "locked", disabled: true })]);
    const locked = merged.find((m) => m.flag === "locked");
    expect(locked).toMatchObject({ disabled: true });
    expect(visibleModelKeys(null, merged)).not.toContain("locked");
    expect(modelOption("locked", merged)).toBe(locked);
  });

  it("resolves a stored preset key to the DISABLED live row, not a static twin", () => {
    const merged = claudeModels([
      row({ flag: null, resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: "sonnet", resolvedModel: "claude-sonnet-5", disabled: true }),
    ]);
    expect(modelOption("sonnet", merged)).toMatchObject({ flag: "sonnet", disabled: true });
  });

  it("keeps a curated disabled key resolving to its greyed row", () => {
    const merged = claudeModels([
      row({ flag: null, resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: "sonnet", resolvedModel: "claude-sonnet-5", disabled: true }),
    ]);
    expect(visibleModelKeys(["sonnet", "haiku"], merged)).toContain("sonnet");
  });

  it("skips a disabled default row when resolving an empty pick", () => {
    const merged = claudeModels([
      row({ flag: null, resolvedModel: "claude-opus-5[1m]", disabled: true }),
      row({ flag: "sonnet", resolvedModel: "claude-sonnet-5" }),
    ]);
    expect(modelOption(undefined, merged)).toMatchObject({ key: "sonnet" });
  });

  it("does not resurrect a disabled listed model as a legacy pin", () => {
    const merged = claudeModels([
      row({ flag: null, resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: "claude-opus-4-8", resolvedModel: "claude-opus-4-8", disabled: true }),
    ]);
    expect(merged.filter((m) => m.flag === "claude-opus-4-8")).toHaveLength(1);
    expect(merged.find((m) => m.flag === "claude-opus-4-8")).toMatchObject({ disabled: true });
  });
});

describe("live catalog canonical keys", () => {
  it("keys a live row by the static preset it re-states", () => {
    const merged = claudeModels([
      row({ flag: null, resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: "haiku", resolvedModel: "claude-haiku-4-5-20251001", supportedEffortLevels: [] }),
      row({ flag: "sonnet[1m]", resolvedModel: "claude-sonnet-5" }),
      row({ flag: "claude-fable-5[1m]", resolvedModel: "claude-fable-5" }),
    ]);
    expect(merged.slice(0, 4).map((m) => m.key)).toEqual(["opus", "haiku", "sonnet-1m", "fable"]);
    expect(modelOption("fable", merged).flag).toBe("claude-fable-5[1m]");
    expect(modelOption("sonnet-1m", merged).flag).toBe("sonnet[1m]");
    expect(modelOption("haiku", merged).efforts).toEqual([]);
  });

  it("keys a genuinely new model by its flag", () => {
    const merged = claudeModels([row({ flag: "claude-nova-6", resolvedModel: "claude-nova-6" })]);
    expect(merged[0].key).toBe("claude-nova-6");
  });

  it("keys an alias and its pinned id independently, in either order", () => {
    const aliasFirst = claudeModels([
      row({ flag: "sonnet", resolvedModel: "claude-sonnet-5" }),
      row({ flag: "claude-sonnet-5", resolvedModel: "claude-sonnet-5" }),
    ]);
    expect(aliasFirst.slice(0, 2).map((m) => m.key)).toEqual(["sonnet", "claude-sonnet-5"]);
    const pinnedFirst = claudeModels([
      row({ flag: "claude-sonnet-5", resolvedModel: "claude-sonnet-5" }),
      row({ flag: "sonnet", resolvedModel: "claude-sonnet-5" }),
    ]);
    expect(pinnedFirst.slice(0, 2).map((m) => m.key)).toEqual(["claude-sonnet-5", "sonnet"]);
  });

  it("never hands the default's key to an explicit row", () => {
    const merged = claudeModels([
      row({ flag: "opus[1m]", resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: null, resolvedModel: "claude-opus-5[1m]" }),
    ]);
    expect(merged[0].key).toBe("opus[1m]");
    expect(merged[1].key).toBe("opus");
  });

  it("keys a row whose flag the default claimed by its resolved id", () => {
    const merged = claudeModels([
      row({ flag: null, resolvedModel: "claude-sonnet-5" }),
      row({ flag: "opus", resolvedModel: "claude-opus-5[1m]" }),
    ]);
    expect(merged[0]).toMatchObject({ key: "opus", flag: null });
    expect(merged[1]).toMatchObject({ key: "claude-opus-5[1m]", flag: "opus" });
  });

  it("drops a row only when every identity it has is already offered", () => {
    const merged = claudeModels([
      row({ flag: null, resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: "claude-opus-5[1m]", resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: "opus", resolvedModel: "claude-opus-5[1m]" }),
    ]);
    expect(merged.filter((m) => !m.unlisted)).toHaveLength(2);
    expect(merged.filter((m) => m.key === "opus")).toHaveLength(1);
  });

  it("dedupes a legacy pin covered by a live row under a different flag spelling", () => {
    const merged = claudeModels([row({ flag: "opus-4-8", resolvedModel: "claude-opus-4-8" })]);
    expect(merged.filter((m) => m.key === "opus-4-8")).toHaveLength(1);
    expect(merged.find((m) => m.key === "opus-4-8")?.flag).toBe("opus-4-8");
  });

  it("emits unique keys for any catalog shape", () => {
    const merged = claudeModels([
      row({ flag: null, resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: "opus", resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: "claude-sonnet-5", resolvedModel: "claude-sonnet-5" }),
      row({ flag: "sonnet", resolvedModel: "claude-sonnet-5" }),
      row({ flag: "claude-fable-5[1m]", resolvedModel: "claude-fable-5" }),
    ]);
    const keys = merged.map((m) => m.key);
    expect(new Set(keys).size).toBe(keys.length);
  });
});

describe("resumeModelFlag", () => {
  it("uses the live default row's own resolved id, not a hardcoded Opus", () => {
    const [def] = claudeModels([row({ flag: null, resolvedModel: "claude-sonnet-5[1m]" })]);
    expect(resumeModelFlag(def)).toBe("claude-sonnet-5[1m]");
  });

  it("falls back to the static assumption without a live catalog", () => {
    expect(resumeModelFlag(modelOption("opus"))).toBe("claude-opus-5");
  });

  it("prefers an explicit flag over the resolved id", () => {
    const [m] = claudeModels([row({ flag: "sonnet", resolvedModel: "claude-sonnet-5" })]);
    expect(resumeModelFlag(m)).toBe("sonnet");
  });
});

describe("pendingModelSync", () => {
  const curated = [
    "opus",
    "sonnet",
    "sonnet-1m",
    "haiku",
    "fable",
    "sonnet-4-6",
    "sonnet-4-6-1m",
    "opus-4-7",
    "opus-4-7-1m",
  ];
  const catalog = claudeModels([
    row({ flag: null, resolvedModel: "claude-opus-5[1m]" }),
    row({ flag: "opus[1m]", resolvedModel: "claude-opus-5[1m]" }),
    row({ flag: "claude-fable-5[1m]", resolvedModel: "claude-fable-5" }),
    row({ flag: "sonnet", resolvedModel: "claude-sonnet-5" }),
    row({ flag: "haiku", resolvedModel: "claude-haiku-4-5-20251001", supportedEffortLevels: [] }),
  ]);

  it("adds models the user was never offered, on a pre-watermark install", () => {
    const sync = pendingModelSync(curated, null, catalog);
    expect(sync?.add).toEqual(["opus[1m]", "opus-4-8", "opus-4-8-1m"]);
  });

  it("never resurrects a model the user removed on purpose", () => {
    const sync = pendingModelSync(curated, null, catalog);
    for (const dropped of ["opus-4-6", "opus-4-6-1m", "opus-4-1"]) {
      expect(sync?.add).not.toContain(dropped);
    }
  });

  it("adds nothing for an uncurated list, but still advances the watermark", () => {
    const sync = pendingModelSync(null, null, catalog);
    expect(sync?.add).toEqual([]);
    expect(sync?.seen).toContain("opus[1m]");
  });

  it("is a no-op once the watermark covers everything on offer", () => {
    const seen = pendingModelSync(curated, null, catalog)!.seen;
    expect(pendingModelSync(curated, seen, catalog)).toBeNull();
    expect(pendingModelSync(null, seen, catalog)).toBeNull();
  });

  it("carries the removal memory forward across two rounds of new models", () => {
    const first = pendingModelSync(curated, null, catalog)!;
    const kept = [...curated, "opus-4-8", "opus-4-8-1m"];
    const later = [...catalog, { key: "opus-6", label: "Opus 6", flag: "opus6", contextWindow: 1_000_000 }];
    const second = pendingModelSync(kept, first.seen, later)!;
    expect(second.add).toEqual(["opus-6"]);
    expect(second.add).not.toContain("opus[1m]");
  });

  it("neither offers nor marks seen an org-disabled model", () => {
    const restricted = claudeModels([
      row({ flag: null, resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: "claude-nova-6", resolvedModel: "claude-nova-6", disabled: true }),
    ]);
    const sync = pendingModelSync(curated, null, restricted);
    expect(sync?.add).not.toContain("claude-nova-6");
    expect(sync?.seen).not.toContain("claude-nova-6");
    const relented = claudeModels([
      row({ flag: null, resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: "claude-nova-6", resolvedModel: "claude-nova-6" }),
    ]);
    expect(pendingModelSync(curated, sync!.seen, relented)?.add).toContain("claude-nova-6");
  });
});

describe("effortsForModel", () => {
  it("offers a live model's own tiers, plus ultracode when it has xhigh", () => {
    const [opus] = claudeModels([row({ flag: "opus[1m]" })]);
    expect(effortsForModel(opus).map((e) => e.key)).toEqual([
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
      "ultracode",
    ]);
  });

  it("offers nothing for a model that takes no --effort", () => {
    const [haiku] = claudeModels([row({ flag: "haiku", supportedEffortLevels: [] })]);
    expect(effortsForModel(haiku)).toEqual([]);
  });

  it("omits ultracode when the model stops below xhigh", () => {
    const [m] = claudeModels([row({ flag: "x", supportedEffortLevels: ["low", "medium", "high"] })]);
    expect(effortsForModel(m).map((e) => e.key)).toEqual(["low", "medium", "high"]);
    expect(modelSupportsUltracode(m)).toBe(false);
  });

  it("labels a tier it hasn't seen instead of silently calling it High", () => {
    const [m] = claudeModels([row({ flag: "x", supportedEffortLevels: ["low", "hyperdrive"] })]);
    expect(effortsForModel(m).map((e) => e.label)).toEqual(["Low", "hyperdrive"]);
  });

  it("falls back to the full ladder for a static row", () => {
    expect(effortsForModel(modelOption("opus-4-7")).map((e) => e.key)).toEqual(
      EFFORTS.map((e) => e.key),
    );
  });
});

describe("ultracode capability guard", () => {
  it("exposes ultracode as the last effort level", () => {
    const ultra = EFFORTS.find((e) => e.key === "ultracode");
    expect(ultra).toEqual({ key: "ultracode", label: "Ultracode" });
  });

  it("allows ultracode on xhigh-capable models", () => {
    for (const key of [
      "opus",
      "sonnet",
      "sonnet-1m",
      "fable",
      "opus-4-8",
      "opus-4-8-1m",
      "opus-4-7",
      "opus-4-7-1m",
    ]) {
      expect(modelSupportsUltracode(modelOption(key))).toBe(true);
    }
  });

  it("blocks ultracode on models without xhigh", () => {
    for (const key of ["haiku", "sonnet-4-6", "sonnet-4-6-1m", "opus-4-6", "opus-4-6-1m", "opus-4-1"]) {
      expect(modelSupportsUltracode(modelOption(key))).toBe(false);
    }
  });

  it("treats custom/unknown model ids as unsupported", () => {
    expect(modelSupportsUltracode(modelOption("claude-3-5-haiku-latest"))).toBe(false);
  });

  it("downgrades ultracode to high on an incompatible model", () => {
    expect(effectiveEffort("ultracode", modelOption("haiku"))?.key).toBe("high");
    expect(effectiveEffort("ultracode", modelOption("opus-4-6"))?.key).toBe("high");
  });

  it("keeps ultracode on a compatible model", () => {
    expect(effectiveEffort("ultracode", modelOption("opus"))?.key).toBe("ultracode");
    expect(effectiveEffort("ultracode", modelOption("sonnet"))?.key).toBe("ultracode");
  });

  it("leaves standard effort levels untouched regardless of model", () => {
    expect(effectiveEffort("low", modelOption("opus"))?.key).toBe("low");
    expect(effectiveEffort("xhigh", modelOption("opus"))?.key).toBe("xhigh");
    expect(effectiveEffort(undefined, modelOption("haiku"))?.key).toBe("high");
  });
});

describe("effectiveEffort against a live tier list", () => {
  const liveRow = (levels: string[]) =>
    claudeModels([row({ flag: "m", supportedEffortLevels: levels })])[0];

  it("sends no --effort at all to a model that takes none", () => {
    expect(effectiveEffort("high", liveRow([]))).toBeNull();
    expect(effectiveEffort(undefined, liveRow([]))).toBeNull();
    expect(effectiveEffort("ultracode", liveRow([]))).toBeNull();
  });

  it("keeps a stored tier the model supports", () => {
    expect(effectiveEffort("max", liveRow(["low", "medium", "high", "xhigh", "max"]))?.key).toBe(
      "max",
    );
  });

  it("keeps a live-reported tier the static ladder can't name", () => {
    expect(effectiveEffort("ultra", liveRow(["low", "medium", "high", "ultra"]))?.key).toBe(
      "ultra",
    );
  });

  it("clamps a stored tier the model rejects down to the default", () => {
    expect(effectiveEffort("max", liveRow(["low", "medium", "high"]))?.key).toBe("high");
  });

  it("clamps to the model's TOP tier when even the default is unsupported", () => {
    expect(effectiveEffort("max", liveRow(["low", "medium"]))?.key).toBe("medium");
    expect(effectiveEffort("max", liveRow(["hyperdrive"]))?.key).toBe("hyperdrive");
  });

  it("clamps ultracode's downgrade too", () => {
    expect(effectiveEffort("ultracode", liveRow(["low", "medium", "high"]))?.key).toBe("high");
    expect(effectiveEffort("ultracode", liveRow(["low", "medium"]))?.key).toBe("medium");
    expect(effectiveEffort("ultracode", liveRow(["low", "medium", "high", "xhigh"]))?.key).toBe(
      "ultracode",
    );
  });
});
