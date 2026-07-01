import { describe, expect, it } from "vitest";

import { EFFORTS, effectiveEffort, modelOption, modelSupportsUltracode } from "./models";

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
