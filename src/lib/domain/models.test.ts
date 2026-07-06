import { describe, expect, it } from "vitest";

import {
  apiContextWindow,
  EFFORTS,
  effectiveEffort,
  modelOption,
  modelSupportsUltracode,
} from "./models";

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
