import { describe, expect, it } from "vitest";

import type { AppSettings } from "@/lib/bindings";
import { quickSettings, withConfirmQuit, withDefaultHarness, withNotify } from "./settingsPatches";

describe("quickSettings", () => {
  it("reads the server defaults when a slice is absent", () => {
    expect(quickSettings({})).toEqual({ defaultHarness: "claude", notify: true, confirmQuit: true });
  });

  it("reads stored values", () => {
    const s: AppSettings = {
      providers: { defaultHarness: "codex" },
      notifications: { enabled: false, sound: true },
      quit: { confirm: false },
    };
    expect(quickSettings(s)).toEqual({ defaultHarness: "codex", notify: false, confirmQuit: false });
  });
});

describe("patches", () => {
  it("toggling notify keeps the sound choice", () => {
    expect(withNotify({ notifications: { enabled: true, sound: false } }, false).notifications).toEqual({
      enabled: false,
      sound: false,
    });
    expect(withNotify({}, false).notifications).toEqual({ enabled: false, sound: true });
  });

  it("changing the default harness keeps every provider's own settings", () => {
    const s: AppSettings = { providers: { defaultHarness: "claude", codex: { defaultModel: "gpt-5.5" } } };
    const next = withDefaultHarness(s, "codex");
    expect(next.providers?.defaultHarness).toBe("codex");
    expect(next.providers?.codex?.defaultModel).toBe("gpt-5.5");
  });

  it("leaves unrelated slices untouched", () => {
    const s: AppSettings = { devTools: true, git: { automerge: true } };
    const next = withConfirmQuit(s, false);
    expect(next.quit).toEqual({ confirm: false });
    expect(next.devTools).toBe(true);
    expect(next.git).toBe(s.git);
  });
});
