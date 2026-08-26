// @vitest-environment jsdom
import { cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClientProvider } from "@tanstack/react-query";
import { createElement, type ReactNode } from "react";

import type { Bootstrap, ClaudeSettings } from "@/lib/bindings";

const cmds = vi.hoisted(() => ({
  getBootstrap: vi.fn(() => new Promise(() => {})),
  claudeSyncModels: vi.fn((_add: string[], _seen: string[]) =>
    Promise.resolve({ status: "ok", data: null }),
  ),
}));
vi.mock("@/lib/bindings", () => ({ commands: cmds }));

import { queryClient } from "@/lib/queryClient";
import { useSyncClaudeModels } from "@/lib/queries";
import { claudeModels, knownModelKeys, type ModelOption } from "@/lib/domain/models";

const STATIC = claudeModels(null);
const LIVE = claudeModels([
  {
    flag: null,
    resolvedModel: "claude-opus-5[1m]",
    displayName: "Default",
    description: null,
    supportedEffortLevels: ["low", "medium", "high", "xhigh", "max"],
    supportsFastMode: true,
    disabled: false,
  },
  {
    flag: "claude-nova-6",
    resolvedModel: "claude-nova-6",
    displayName: "Nova",
    description: null,
    supportedEffortLevels: ["low", "medium", "high"],
    supportsFastMode: false,
    disabled: false,
  },
]);

function seedBootstrap(claude: ClaudeSettings) {
  queryClient.setQueryData(
    ["bootstrap"],
    { settings: { providers: { claude } } } as unknown as Bootstrap,
  );
}

const wrapper = ({ children }: { children: ReactNode }) =>
  createElement(QueryClientProvider, { client: queryClient }, children);

type Props = { catalog: readonly ModelOption[]; settled: boolean; live: boolean };
const renderSync = (initialProps: Props) =>
  renderHook(({ catalog, settled, live }: Props) => useSyncClaudeModels(catalog, settled, live), {
    wrapper,
    initialProps,
  });

afterEach(() => {
  cleanup();
  queryClient.clear();
  vi.clearAllMocks();
});

describe("useSyncClaudeModels", () => {
  const curated = { visibleModels: ["opus"], seenModelKeys: null } as unknown as ClaudeSettings;

  it("does nothing until the probe settles", () => {
    seedBootstrap(curated);
    const { rerender } = renderSync({ catalog: STATIC, settled: false, live: false });
    expect(cmds.claudeSyncModels).not.toHaveBeenCalled();
    rerender({ catalog: STATIC, settled: true, live: false });
    expect(cmds.claudeSyncModels).toHaveBeenCalledTimes(1);
  });

  it("syncs once against the live catalog, then latches", async () => {
    seedBootstrap(curated);
    const { rerender } = renderSync({ catalog: LIVE, settled: true, live: true });
    await waitFor(() => expect(cmds.claudeSyncModels).toHaveBeenCalledTimes(1));
    const [add] = cmds.claudeSyncModels.mock.calls[0];
    expect(add).toContain("claude-nova-6");
    rerender({ catalog: LIVE, settled: true, live: true });
    expect(cmds.claudeSyncModels).toHaveBeenCalledTimes(1);
  });

  it("lets a live catalog re-sync after an error-path static sync", async () => {
    seedBootstrap(curated);
    const { rerender } = renderSync({ catalog: STATIC, settled: true, live: false });
    await waitFor(() => expect(cmds.claudeSyncModels).toHaveBeenCalledTimes(1));
    expect(cmds.claudeSyncModels.mock.calls[0][0]).not.toContain("claude-nova-6");
    rerender({ catalog: LIVE, settled: true, live: true });
    await waitFor(() => expect(cmds.claudeSyncModels).toHaveBeenCalledTimes(2));
    expect(cmds.claudeSyncModels.mock.calls[1][0]).toContain("claude-nova-6");
    rerender({ catalog: LIVE, settled: true, live: true });
    expect(cmds.claudeSyncModels).toHaveBeenCalledTimes(2);
  });

  it("writes nothing when the watermark is already current", () => {
    seedBootstrap({
      visibleModels: ["opus"],
      seenModelKeys: knownModelKeys(LIVE),
    } as unknown as ClaudeSettings);
    renderSync({ catalog: LIVE, settled: true, live: true });
    expect(cmds.claudeSyncModels).not.toHaveBeenCalled();
  });
});
