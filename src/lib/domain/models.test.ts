import { describe, expect, it } from "vitest";

import type { ClaudeModel } from "@/lib/bindings";

import {
  apiContextWindow,
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

/** A `list_models` row with the defaults most tests don't care about. */
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
    // Live model/list data: Sol goes to `ultra`, Luna stops at `max`. Offering a
    // tier the chosen model rejects would fail the turn.
    expect(codexEfforts(["low", "medium", "high", "xhigh", "max", "ultra"]).map((e) => e.key))
      .toEqual(["low", "medium", "high", "xhigh", "max", "ultra"]);
    expect(codexEfforts(["low", "medium", "high", "xhigh", "max"]).map((e) => e.key))
      .toEqual(["low", "medium", "high", "xhigh", "max"]);
  });

  it("falls back to the tiers every Codex model supports", () => {
    // Before the model list loads. `ultracode` is Claude-only and must never
    // appear; `max`/`ultra` aren't universal so they stay out of the fallback.
    const keys = CODEX_EFFORTS.map((e) => e.key);
    expect(keys).toEqual(["low", "medium", "high", "xhigh"]);
    expect(codexEfforts(undefined).map((e) => e.key)).toEqual(keys);
    expect(codexEfforts([]).map((e) => e.key)).toEqual(keys);
  });

  it("labels a tier it hasn't seen instead of dropping it", () => {
    // A new Codex tier should still be pickable, just unprettified.
    expect(codexEfforts(["low", "hyperdrive"]).map((e) => e.label)).toEqual(["Low", "hyperdrive"]);
  });

  it("resolves a stored key against what the model actually supports", () => {
    const sol = ["low", "medium", "high", "xhigh", "max", "ultra"];
    expect(resolveCodexEffort("ultra", sol).key).toBe("ultra");
    // Switching Sol → Luna drops `ultra`; the stale pick must not ride along.
    expect(resolveCodexEffort("ultra", ["low", "medium", "high", "xhigh", "max"]).key)
      .toBe("medium");
    // Unset, or a Claude-only level carried over from another harness.
    expect(resolveCodexEffort(undefined, sol).key).toBe("medium");
    expect(resolveCodexEffort("ultracode", sol).key).toBe("medium");
    expect(resolveCodexEffort(null, undefined).key).toBe("medium");
  });
});

describe("resolveCodexModelIds", () => {
  const probed = [{ id: "gpt-5.4-codex" }, { id: "gpt-5.4" }, { id: "gpt-5.4-mini" }];

  it("offers every probed model when the user hasn't curated a selection", () => {
    // The point of the change: signed in + no explicit picks ⇒ all models, the
    // dynamic-list twin of Claude showing all its presets by default.
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
    // Resuming onto the null-flag default needs an explicit id.
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
    // Callers keep their existing label rather than render something wrong.
    expect(modelDisplayName("anthropic/claude-opus-5")).toBeUndefined();
    expect(modelDisplayName("ollama/qwen2.5-coder")).toBeUndefined();
    expect(modelDisplayName("gpt-5.6-sol")).toBeUndefined();
    expect(modelDisplayName(null)).toBeUndefined();
    expect(modelDisplayName("")).toBeUndefined();
  });
});

describe("live catalog merge", () => {
  /** Verbatim shape from `list_models` (claude 2.1.219), trimmed. */
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
    // No CLI, signed out, or an older CLI without the control request.
    expect(claudeModels(null)).toEqual(MODELS);
    expect(claudeModels([])).toEqual(MODELS);
  });

  it("prefers live rows and keeps the legacy pins the CLI omits", () => {
    const merged = claudeModels(live);
    // Versioned names, not the CLI's vaguer "Default (recommended)" / "Sonnet".
    expect(merged.slice(0, 3).map((m) => m.label)).toEqual(["Opus 5", "Sonnet 5", "Haiku 4.5"]);
    // The no-`--model` row says which model it resolves to AND that it's default.
    expect(merged[0].hint).toBe("1M · default");
    // Unlisted ids still launch (`--model` is validated lazily), so they stay,
    // flagged, not hidden.
    const legacy = merged.find((m) => m.key === "opus-4-7");
    expect(legacy).toMatchObject({ flag: "claude-opus-4-7", unlisted: true });
    // A static row the catalog DOES cover must not appear twice.
    expect(merged.filter((m) => m.flag === "sonnet")).toHaveLength(1);
  });

  it("keys the null-flag row as the default so stored settings still resolve", () => {
    const merged = claudeModels(live);
    expect(modelOption("opus", merged)).toMatchObject({ flag: null, label: "Opus 5" });
    // A key the live catalog dropped falls through to the static presets rather
    // than being mistaken for a custom id (which would pass "sonnet-1m" as --model).
    expect(modelOption("sonnet-1m", merged).flag).toBe("sonnet[1m]");
  });

  it("reads the 1M window off the flag or the resolved id", () => {
    const merged = claudeModels(live);
    // The default row's flag is null; only `resolvedModel` carries the [1m].
    expect(merged[0].contextWindow).toBe(1_000_000);
    expect(merged[1].contextWindow).toBe(200_000);
  });

  it("keeps org-disabled models, marked, so stored keys can't resurrect them", () => {
    // Filtering them out entirely let `modelOption` fall through to a LAUNCHABLE
    // static row for the same model, the catalog said no, the app said yes.
    const merged = claudeModels([...live, row({ flag: "locked", disabled: true })]);
    const locked = merged.find((m) => m.flag === "locked");
    expect(locked).toMatchObject({ disabled: true });
    // Not offered anywhere by default…
    expect(visibleModelKeys(null, merged)).not.toContain("locked");
    // …but a curated list that already names it resolves to the greyed row.
    expect(modelOption("locked", merged)).toBe(locked);
  });

  it("resolves a stored preset key to the DISABLED live row, not a static twin", () => {
    // Org policy disables Sonnet: the live row canonical-keys to "sonnet", so a
    // stored default of "sonnet" must land on it (greyed), never the launchable
    // static preset behind it.
    const merged = claudeModels([
      row({ flag: null, resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: "sonnet", resolvedModel: "claude-sonnet-5", disabled: true }),
    ]);
    expect(modelOption("sonnet", merged)).toMatchObject({ flag: "sonnet", disabled: true });
  });

  it("keeps a curated disabled key resolving to its greyed row", () => {
    // Curation is the user's list; a disabled member stays on it (greyed with
    // the reason) rather than silently vanishing.
    const merged = claudeModels([
      row({ flag: null, resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: "sonnet", resolvedModel: "claude-sonnet-5", disabled: true }),
    ]);
    expect(visibleModelKeys(["sonnet", "haiku"], merged)).toContain("sonnet");
  });

  it("skips a disabled default row when resolving an empty pick", () => {
    // The implicit selection (no stored key) must never be a row the CLI would
    // refuse to launch.
    const merged = claudeModels([
      row({ flag: null, resolvedModel: "claude-opus-5[1m]", disabled: true }),
      row({ flag: "sonnet", resolvedModel: "claude-sonnet-5" }),
    ]);
    expect(modelOption(undefined, merged)).toMatchObject({ key: "sonnet" });
  });

  it("does not resurrect a disabled listed model as a legacy pin", () => {
    // A disabled row's flag still counts as "listed", the legacy tail must not
    // re-offer the same model unmarked.
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
      // Flag match: the CLI uses the same alias our preset launches with.
      row({ flag: "haiku", resolvedModel: "claude-haiku-4-5-20251001", supportedEffortLevels: [] }),
      row({ flag: "sonnet[1m]", resolvedModel: "claude-sonnet-5" }),
      // Name+window match: the CLI spells Fable's flag as a full id, but it's
      // the same model our "fable" preset launches.
      row({ flag: "claude-fable-5[1m]", resolvedModel: "claude-fable-5" }),
    ]);
    expect(merged.slice(0, 4).map((m) => m.key)).toEqual(["opus", "haiku", "sonnet-1m", "fable"]);
    // Stored settings written before the live catalog keep resolving to the
    // LIVE rows (with their effort lists), not the static fallbacks.
    expect(modelOption("fable", merged).flag).toBe("claude-fable-5[1m]");
    expect(modelOption("sonnet-1m", merged).flag).toBe("sonnet[1m]");
    expect(modelOption("haiku", merged).efforts).toEqual([]);
  });

  it("keys a genuinely new model by its flag", () => {
    const merged = claudeModels([row({ flag: "claude-nova-6", resolvedModel: "claude-nova-6" })]);
    expect(merged[0].key).toBe("claude-nova-6");
  });

  it("keys an alias and its pinned id independently, in either order", () => {
    // Flag identity outranks a name match, so a pinned-id row listed FIRST
    // must not rob the alias row of its own key, stored settings resolve by key.
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
    // An explicit opus row is not the null-flag default, even when it resolves
    // to the same model the default launches.
    const merged = claudeModels([
      row({ flag: "opus[1m]", resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: null, resolvedModel: "claude-opus-5[1m]" }),
    ]);
    expect(merged[0].key).toBe("opus[1m]");
    expect(merged[1].key).toBe("opus");
  });

  it("keys a row whose flag the default claimed by its resolved id", () => {
    // A literal "opus" alias row is a REAL offering when the default resolves
    // elsewhere. It must survive under its resolved id, still selectable.
    const merged = claudeModels([
      row({ flag: null, resolvedModel: "claude-sonnet-5" }),
      row({ flag: "opus", resolvedModel: "claude-opus-5[1m]" }),
    ]);
    expect(merged[0]).toMatchObject({ key: "opus", flag: null });
    expect(merged[1]).toMatchObject({ key: "claude-opus-5[1m]", flag: "opus" });
  });

  it("drops a row only when every identity it has is already offered", () => {
    // Default + pinned id + literal alias, all Opus 5 1M: the alias row's flag
    // AND resolved id are both claimed, so it is a true duplicate spelling.
    const merged = claudeModels([
      row({ flag: null, resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: "claude-opus-5[1m]", resolvedModel: "claude-opus-5[1m]" }),
      row({ flag: "opus", resolvedModel: "claude-opus-5[1m]" }),
    ]);
    expect(merged.filter((m) => !m.unlisted)).toHaveLength(2);
    expect(merged.filter((m) => m.key === "opus")).toHaveLength(1);
  });

  it("dedupes a legacy pin covered by a live row under a different flag spelling", () => {
    // The live row name-claims the legacy key "opus-4-8" with an alias flag;
    // the legacy tail must not re-add the same key under its full-id flag.
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
    // A user whose CLI default resolves elsewhere must resume onto THAT model.
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
  // The account whose curated list predates the catalog: they kept 9 of the 12
  // keys that existed then, deliberately dropping Opus 4.6/4.6-1M/4.1.
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
    // New since they curated: the explicit 1M Opus row and the Opus 4.8 pins.
    // The live Fable row canonical-keys to "fable", already theirs, NOT
    // re-added under its new flag spelling; `opus`/`sonnet`/`haiku` likewise.
    expect(sync?.add).toEqual(["opus[1m]", "opus-4-8", "opus-4-8-1m"]);
  });

  it("never resurrects a model the user removed on purpose", () => {
    const sync = pendingModelSync(curated, null, catalog);
    // The whole point of the watermark: these existed when they curated and were
    // dropped, so a "refresh" must not hand them back.
    for (const dropped of ["opus-4-6", "opus-4-6-1m", "opus-4-1"]) {
      expect(sync?.add).not.toContain(dropped);
    }
  });

  it("adds nothing for an uncurated list, but still advances the watermark", () => {
    // `null` already shows everything; writing an explicit list would silently
    // convert them to a curated user and freeze them out of the NEXT model.
    const sync = pendingModelSync(null, null, catalog);
    expect(sync?.add).toEqual([]);
    expect(sync?.seen).toContain("opus[1m]");
  });

  it("is a no-op once the watermark covers everything on offer", () => {
    const seen = pendingModelSync(curated, null, catalog)!.seen;
    expect(pendingModelSync(curated, seen, catalog)).toBeNull();
    // And so stays a no-op on every later launch, no write, no churn.
    expect(pendingModelSync(null, seen, catalog)).toBeNull();
  });

  it("carries the removal memory forward across two rounds of new models", () => {
    const first = pendingModelSync(curated, null, catalog)!;
    // A later release adds one model; the user meanwhile removed the explicit
    // 1M Opus row. It must not come back, and only the genuinely-new key is added.
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
    // If policy later relents, the model is still "new" and reaches the user.
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
    // Haiku reports an empty tier list, different from "we don't know", and the
    // reason the composer hides the picker entirely rather than offering tiers
    // the CLI ignores.
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
    // `efforts` undefined ⇒ unknown, so keep offering everything.
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
    // A typed-in custom model id (modelOption's fall-through) is never in the set.
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
    // Unknown/undefined effort still falls back to the default (high). The
    // STATIC haiku row carries no tier list, so nothing clamps it here.
    expect(effectiveEffort(undefined, modelOption("haiku"))?.key).toBe("high");
  });
});

describe("effectiveEffort against a live tier list", () => {
  const liveRow = (levels: string[]) =>
    claudeModels([row({ flag: "m", supportedEffortLevels: levels })])[0];

  it("sends no --effort at all to a model that takes none", () => {
    // Live Haiku: the picker is hidden AND the launch flag must be omitted,
    // hiding the menu while still sending the stale stored tier was the bug.
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
    // effortsForModel offers new CLI tiers as pickable; the pick must survive
    // to launch instead of effortOption's unknown→High fallback eating it.
    expect(effectiveEffort("ultra", liveRow(["low", "medium", "high", "ultra"]))?.key).toBe(
      "ultra",
    );
  });

  it("clamps a stored tier the model rejects down to the default", () => {
    expect(effectiveEffort("max", liveRow(["low", "medium", "high"]))?.key).toBe("high");
  });

  it("clamps to the model's TOP tier when even the default is unsupported", () => {
    // Always an explicit supported tier, never an omitted flag: `--resume`
    // inherits the old session's effort when --effort is omitted, so null here
    // would silently carry a rejected tier onto the new model.
    expect(effectiveEffort("max", liveRow(["low", "medium"]))?.key).toBe("medium");
    // Tiers we can't rank still resolve to something the model reported.
    expect(effectiveEffort("max", liveRow(["hyperdrive"]))?.key).toBe("hyperdrive");
  });

  it("clamps ultracode's downgrade too", () => {
    // Ultracode on a model without xhigh falls to high, then clamps like any
    // other tier.
    expect(effectiveEffort("ultracode", liveRow(["low", "medium", "high"]))?.key).toBe("high");
    expect(effectiveEffort("ultracode", liveRow(["low", "medium"]))?.key).toBe("medium");
    // With xhigh present, ultracode rides as itself (the driver translates it).
    expect(effectiveEffort("ultracode", liveRow(["low", "medium", "high", "xhigh"]))?.key).toBe(
      "ultracode",
    );
  });
});

describe("apiContextWindow", () => {
  const windows = new Map([
    ["claude-fable-5", 1_000_000],
    ["claude-opus-4-8", 1_000_000],
    ["claude-haiku-4-5-20251001", 200_000],
  ]);

  it("returns the API window for a resolved bare id", () => {
    expect(apiContextWindow("claude-fable-5", windows)).toBe(1_000_000);
  });

  it("strips a [1m] variant suffix before lookup (API keys on bare ids)", () => {
    expect(apiContextWindow("claude-opus-4-8[1m]", windows)).toBe(1_000_000);
  });

  it("returns undefined for ids the catalog doesn't cover, so the caller falls back", () => {
    // Unknown model, an OpenCode/Ollama id, or a null/absent init model.
    expect(apiContextWindow("claude-opus-4-7", windows)).toBeUndefined();
    expect(apiContextWindow("ollama/qwen2.5-coder", windows)).toBeUndefined();
    expect(apiContextWindow(null, windows)).toBeUndefined();
    expect(apiContextWindow(undefined, windows)).toBeUndefined();
  });
});
