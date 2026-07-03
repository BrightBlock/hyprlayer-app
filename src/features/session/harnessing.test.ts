import { describe, expect, it } from "vitest";

import type { AppSettings } from "../../lib/bindings";
import {
  actionAgentDefaults,
  actionLaunchArgs,
  effectiveHarnessKey,
  lockedSessionProvider,
  modelProvider,
} from "./harnessing";

const withProviders = (providers: AppSettings["providers"]): AppSettings =>
  ({ providers }) as AppSettings;

describe("modelProvider", () => {
  it("returns the id prefix for OpenCode models", () => {
    expect(modelProvider("opencode", "openrouter/moonshotai/kimi-k2.7-code")).toBe("openrouter");
    expect(modelProvider("opencode", "opencode/glm-5-free")).toBe("opencode");
    expect(modelProvider("opencode", "opencode-go/deepseek-v4-flash")).toBe("opencode-go");
    expect(modelProvider("opencode", "ollama/qwen2.5-coder:latest")).toBe("ollama");
  });

  it("treats every Claude model as the single `claude` provider", () => {
    // All Anthropic models share one endpoint, so switching among them mid-session
    // never crosses a provider — that's why Claude switching is seamless.
    expect(modelProvider("claude", null)).toBe("claude");
    expect(modelProvider("claude", "ignored")).toBe("claude");
  });

  it("is null for an OpenCode harness with no model resolved yet", () => {
    expect(modelProvider("opencode", null)).toBeNull();
    expect(modelProvider("opencode", undefined)).toBeNull();
    expect(modelProvider("opencode", "")).toBeNull();
  });
});

describe("lockedSessionProvider", () => {
  it("locks nothing before the session starts (free choice on a fresh tab)", () => {
    expect(lockedSessionProvider(false, "opencode", "openrouter/x")).toBeNull();
    expect(lockedSessionProvider(false, "claude", null)).toBeNull();
  });

  it("locks to the started session's underlying provider", () => {
    // Kimi via OpenRouter → locked to `openrouter`; Big Pickle (Zen, `opencode`)
    // is a different provider and must be blocked.
    const locked = lockedSessionProvider(true, "opencode", "openrouter/moonshotai/kimi-k2.7-code");
    expect(locked).toBe("openrouter");
    expect(modelProvider("opencode", "openrouter/auto-router")).toBe(locked); // same provider → allowed
    expect(modelProvider("opencode", "opencode/big-pickle")).not.toBe(locked); // cross provider → blocked
  });

  it("locks a started Claude session to `claude` (so it can't jump to OpenCode)", () => {
    const locked = lockedSessionProvider(true, "claude", null);
    expect(locked).toBe("claude");
    expect(modelProvider("opencode", "openrouter/x")).not.toBe(locked);
  });
});

describe("actionAgentDefaults", () => {
  it("defaults to Claude sonnet/medium when nothing is configured", () => {
    const expected = { harness: "claude", model: "sonnet", effort: "medium" };
    expect(actionAgentDefaults(undefined)).toEqual(expected);
    expect(actionAgentDefaults(withProviders({}))).toEqual(expected);
  });

  it("honours the configured Claude commit model + effort", () => {
    const s = withProviders({ claude: { commitModel: "haiku", commitEffort: "low" } });
    expect(actionAgentDefaults(s)).toEqual({ harness: "claude", model: "haiku", effort: "low" });
  });

  it("follows an OpenCode default harness, using its commit model (no effort)", () => {
    const s = withProviders({
      defaultHarness: "opencode",
      opencode: { commitModel: "opencode/glm-5-free", selectedModels: [] },
    });
    expect(actionAgentDefaults(s)).toEqual({
      harness: "opencode",
      model: "opencode/glm-5-free",
      effort: null,
    });
  });

  it("falls back through the OpenCode default then first-selected model", () => {
    // No commitModel → opencode.defaultModel.
    expect(
      actionAgentDefaults(
        withProviders({
          defaultHarness: "opencode",
          opencode: { defaultModel: "openrouter/x", selectedModels: ["openrouter/x"] },
        }),
      ),
    ).toEqual({ harness: "opencode", model: "openrouter/x", effort: null });

    // No commit/default → the first exposed OpenCode model.
    expect(
      actionAgentDefaults(
        withProviders({
          defaultHarness: "opencode",
          opencode: { selectedModels: ["opencode/big-pickle"] },
        }),
      ),
    ).toEqual({ harness: "opencode", model: "opencode/big-pickle", effort: null });
  });

  it("runs an Ollama default on the OpenCode harness with its own commit model", () => {
    // Ollama is its own default, but launches on the OpenCode harness.
    expect(
      actionAgentDefaults(
        withProviders({
          defaultHarness: "ollama",
          ollama: {
            commitModel: "ollama/qwen2.5-coder:7b",
            selectedModels: ["qwen2.5-coder:7b"],
          },
        }),
      ),
    ).toEqual({ harness: "opencode", model: "ollama/qwen2.5-coder:7b", effort: null });

    // No commit model → the first selected, surfaced as an `ollama/<id>` id.
    expect(
      actionAgentDefaults(
        withProviders({
          defaultHarness: "ollama",
          ollama: { selectedModels: ["qwen2.5-coder:7b"] },
        }),
      ),
    ).toEqual({ harness: "opencode", model: "ollama/qwen2.5-coder:7b", effort: null });
  });
});

describe("effectiveHarnessKey", () => {
  it("maps OpenCode and Ollama to the OpenCode harness, else Claude", () => {
    expect(effectiveHarnessKey("opencode")).toBe("opencode");
    expect(effectiveHarnessKey("ollama")).toBe("opencode");
    expect(effectiveHarnessKey("claude")).toBe("claude");
    expect(effectiveHarnessKey(undefined)).toBe("claude");
    expect(effectiveHarnessKey(null)).toBe("claude");
  });
});

describe("actionLaunchArgs", () => {
  it("translates a Claude model key to its --model flag, keeping effort", () => {
    // "sonnet" key → "sonnet" flag; "opus" → null (claude's own built-in default).
    expect(actionLaunchArgs({ harness: "claude", model: "sonnet", effort: "medium" })).toEqual({
      harness: "claude",
      model: "sonnet",
      effort: "medium",
    });
    expect(actionLaunchArgs({ harness: "claude", model: "opus", effort: "high" })).toEqual({
      harness: "claude",
      model: null,
      effort: "high",
    });
  });

  it("passes an OpenCode provider/model id through, with no effort", () => {
    expect(actionLaunchArgs({ harness: "opencode", model: "openrouter/x", effort: null })).toEqual({
      harness: "opencode",
      model: "openrouter/x",
      effort: null,
    });
  });
});
