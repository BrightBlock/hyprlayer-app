import { describe, expect, it } from "vitest";

import { lockedSessionProvider, modelProvider } from "./harnessing";

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
