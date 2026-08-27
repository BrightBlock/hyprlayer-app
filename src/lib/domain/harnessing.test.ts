import { describe, expect, it } from "vitest";

import type { AppSettings } from "@/lib/bindings";
import type { ModelOption } from "./models";
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
    const locked = lockedSessionProvider(true, "opencode", "openrouter/moonshotai/kimi-k2.7-code");
    expect(locked).toBe("openrouter");
    expect(modelProvider("opencode", "openrouter/auto-router")).toBe(locked);
    expect(modelProvider("opencode", "opencode/big-pickle")).not.toBe(locked);
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
    expect(
      actionAgentDefaults(
        withProviders({
          defaultHarness: "opencode",
          opencode: { defaultModel: "openrouter/x", selectedModels: ["openrouter/x"] },
        }),
      ),
    ).toEqual({ harness: "opencode", model: "openrouter/x", effort: null });

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
    expect(effectiveHarnessKey("codex")).toBe("codex");
  });
});

describe("actionLaunchArgs", () => {
  it("translates a Claude model key to its --model flag, keeping effort", () => {
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

  it("clamps the effort to what the live catalog says the model accepts", () => {
    const catalog: ModelOption[] = [
      { key: "haiku", label: "Haiku 4.5", flag: "haiku", contextWindow: 200_000, efforts: [] },
      {
        key: "sonnet",
        label: "Sonnet 5",
        flag: "sonnet",
        contextWindow: 200_000,
        efforts: ["low", "medium", "high"],
      },
    ];
    expect(actionLaunchArgs({ harness: "claude", model: "haiku", effort: "medium" }, catalog))
      .toEqual({ harness: "claude", model: "haiku", effort: null });
    expect(actionLaunchArgs({ harness: "claude", model: "sonnet", effort: "max" }, catalog))
      .toEqual({ harness: "claude", model: "sonnet", effort: "high" });
  });

  it("substitutes the catalog default when the configured model is org-disabled", () => {
    const catalog: ModelOption[] = [
      {
        key: "opus",
        label: "Opus 5",
        flag: null,
        contextWindow: 1_000_000,
        efforts: ["low", "medium", "high"],
      },
      {
        key: "sonnet",
        label: "Sonnet 5",
        flag: "sonnet",
        contextWindow: 200_000,
        efforts: ["low", "medium", "high"],
        disabled: true,
      },
    ];
    expect(actionLaunchArgs({ harness: "claude", model: "sonnet", effort: "medium" }, catalog))
      .toEqual({ harness: "claude", model: null, effort: "medium" });
    const settings = withProviders({
      claude: { defaultModel: "sonnet" },
    } as AppSettings["providers"]);
    expect(newChatLaunchArgs(settings, {}, catalog).model).toBeNull();
  });

  it("passes an OpenCode provider/model id through, with no effort", () => {
    expect(actionLaunchArgs({ harness: "opencode", model: "openrouter/x", effort: null })).toEqual({
      harness: "opencode",
      model: "openrouter/x",
      effort: null,
    });
  });

  it("never launches a legacy Copilot model and falls through to a supported model", () => {
    const settings = withProviders({
      defaultHarness: "opencode",
      opencode: {
        defaultModel: "github-copilot/default",
        commitModel: "GITHUB-COPILOT/commit",
        selectedModels: ["GitHub-Copilot/selected", "openai/gpt-5"],
      },
    });

    expect(
      newChatLaunchArgs(settings, { opencodeModel: "gItHuB-CoPiLoT/shell" }),
    ).toMatchObject({ harness: "opencode", model: "openai/gpt-5", effort: null });
    expect(actionAgentDefaults(settings)).toEqual({
      harness: "opencode",
      model: "openai/gpt-5",
      effort: null,
    });
    expect(
      actionLaunchArgs({ harness: "opencode", model: "github-copilot/direct", effort: null }),
    ).toEqual({ harness: "opencode", model: null, effort: null });
  });

  it("passes a Codex model id through AND keeps its effort", () => {
    expect(
      actionLaunchArgs({ harness: "codex", model: "gpt-5.4-codex", effort: "low" }),
    ).toEqual({ harness: "codex", model: "gpt-5.4-codex", effort: "low" });
  });
});

describe("Codex defaults", () => {
  it("locks a started Codex session to the single `codex` provider", () => {
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

    expect(newChatLaunchArgs(settings, {})).toEqual({
      harness: "codex",
      model: "gpt-5.4-codex",
      effort: "high",
      permissionMode: null,
    });

    expect(
      newChatLaunchArgs(settings, {
        codexModel: "gpt-5.4",
        codexEffort: "low",
        permissionMode: "plan",
      }),
    ).toEqual({ harness: "codex", model: "gpt-5.4", effort: "low", permissionMode: "plan" });
  });

  it("falls back to Codex's middle effort tier, never a Claude-only level", () => {
    const settings = withProviders({
      defaultHarness: "codex",
      codex: { selectedModels: ["m"], defaultEffort: "ultracode" },
    } as AppSettings["providers"]);
    expect(newChatLaunchArgs(settings, {}).effort).toBe("medium");
  });
});
