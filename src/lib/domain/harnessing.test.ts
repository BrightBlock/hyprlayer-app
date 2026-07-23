import { describe, expect, it } from "vitest";

import type { AppSettings } from "@/lib/bindings";
import {
  actionAgentDefaults,
  actionLaunchArgs,
  effectiveHarnessKey,
  lockedSessionProvider,
  modelProvider,
  newChatLaunchArgs,
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

  it("keeps Codex as its own harness", () => {
    // Unlike Ollama (which rides OpenCode), Codex is a real harness of its own.
    expect(effectiveHarnessKey("codex")).toBe("codex");
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

  it("passes a Codex model id through AND keeps its effort", () => {
    // Codex ids need no translation (unlike Claude's keys), but — unlike
    // OpenCode — Codex does take a reasoning effort, so it must not be dropped.
    expect(
      actionLaunchArgs({ harness: "codex", model: "gpt-5.4-codex", effort: "low" }),
    ).toEqual({ harness: "codex", model: "gpt-5.4-codex", effort: "low" });
  });
});

describe("Codex defaults", () => {
  it("locks a started Codex session to the single `codex` provider", () => {
    // All Codex models share one endpoint, so model switching mid-session is
    // always safe — the lock only has to keep it from jumping to another harness.
    expect(modelProvider("codex", null)).toBe("codex");
    expect(lockedSessionProvider(true, "codex", null)).toBe("codex");
  });

  it("runs the action agents on the configured Codex commit model + effort", () => {
    expect(
      actionAgentDefaults(
        withProviders({
          defaultHarness: "codex",
          codex: {
            selectedModels: ["gpt-5.4-codex", "gpt-5.4"],
            defaultModel: "gpt-5.4-codex",
            commitModel: "gpt-5.4",
            commitEffort: "low",
          },
        } as AppSettings["providers"]),
      ),
    ).toEqual({ harness: "codex", model: "gpt-5.4", effort: "low" });
  });

  it("falls back through the Codex default then the first selected model", () => {
    expect(
      actionAgentDefaults(
        withProviders({
          defaultHarness: "codex",
          codex: { selectedModels: ["a", "b"], defaultModel: "b" },
        } as AppSettings["providers"]),
      ),
    ).toEqual({ harness: "codex", model: "b", effort: "medium" });

    expect(
      actionAgentDefaults(
        withProviders({
          defaultHarness: "codex",
          codex: { selectedModels: ["a", "b"] },
        } as AppSettings["providers"]),
      ),
    ).toEqual({ harness: "codex", model: "a", effort: "medium" });

    // Nothing configured at all still names the harness — the backend then lets
    // Codex pick its own default model.
    expect(
      actionAgentDefaults(
        withProviders({ defaultHarness: "codex" } as AppSettings["providers"]),
      ),
    ).toEqual({ harness: "codex", model: null, effort: "medium" });
  });

  it("launches a new Codex chat on the tab's picks, then the settings defaults", () => {
    const settings = withProviders({
      defaultHarness: "codex",
      codex: {
        selectedModels: ["gpt-5.4-codex"],
        defaultModel: "gpt-5.4-codex",
        defaultEffort: "high",
      },
    } as AppSettings["providers"]);

    // No tab picks → the configured defaults. Codex maps our permission mode onto
    // its approval/sandbox pair, so the composer's choice rides along (unlike
    // OpenCode, which has no permission modes).
    expect(newChatLaunchArgs(settings, {})).toEqual({
      harness: "codex",
      model: "gpt-5.4-codex",
      effort: "high",
      permissionMode: null,
    });

    // The tab's own picks win over the settings defaults.
    expect(
      newChatLaunchArgs(settings, {
        codexModel: "gpt-5.4",
        codexEffort: "low",
        permissionMode: "plan",
      }),
    ).toEqual({ harness: "codex", model: "gpt-5.4", effort: "low", permissionMode: "plan" });
  });

  it("falls back to Codex's middle effort tier, never a Claude-only level", () => {
    // `max`/`ultracode` exist on Claude but not Codex — sending one would be
    // rejected by the app-server, so an unrecognized value resolves to `medium`.
    const settings = withProviders({
      defaultHarness: "codex",
      codex: { selectedModels: ["m"], defaultEffort: "ultracode" },
    } as AppSettings["providers"]);
    expect(newChatLaunchArgs(settings, {}).effort).toBe("medium");
  });
});
