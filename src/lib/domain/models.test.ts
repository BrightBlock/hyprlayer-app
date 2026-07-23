import { describe, expect, it } from "vitest";

import {
  apiContextWindow,
  CODEX_EFFORTS,
  codexEfforts,
  resolveCodexEffort,
  EFFORTS,
  effectiveEffort,
  modelOption,
  modelSupportsUltracode,
} from "./models";

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

describe("ultracode capability guard", () => {
  it("exposes ultracode as the last effort level", () => {
    const ultra = EFFORTS.find((e) => e.key === "ultracode");
    expect(ultra).toEqual({ key: "ultracode", label: "Ultracode" });
  });

  it("allows ultracode on xhigh-capable models", () => {
    for (const key of ["opus", "sonnet", "sonnet-1m", "fable", "opus-4-7", "opus-4-7-1m"]) {
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
    expect(effectiveEffort("ultracode", modelOption("haiku")).key).toBe("high");
    expect(effectiveEffort("ultracode", modelOption("opus-4-6")).key).toBe("high");
  });

  it("keeps ultracode on a compatible model", () => {
    expect(effectiveEffort("ultracode", modelOption("opus")).key).toBe("ultracode");
    expect(effectiveEffort("ultracode", modelOption("sonnet")).key).toBe("ultracode");
  });

  it("leaves standard effort levels untouched regardless of model", () => {
    expect(effectiveEffort("low", modelOption("opus")).key).toBe("low");
    expect(effectiveEffort("xhigh", modelOption("opus")).key).toBe("xhigh");
    // Unknown/undefined effort still falls back to the default (high).
    expect(effectiveEffort(undefined, modelOption("haiku")).key).toBe("high");
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
