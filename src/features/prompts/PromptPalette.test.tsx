// @vitest-environment jsdom
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { WorkstreamView } from "@/lib/bindings";
import { ESCAPE_COMMAND_ID, runCommand } from "@/lib/keybindings/registry";

vi.mock("@/lib/queries", () => ({
  useBootstrap: () => ({
    data: {
      settings: { prompts: { custom: [{ id: "p1", name: "Summarize", body: "Summarize this", overrides: {} }] } },
      promptDefaults: { commit: "/commit", ship: "ship" },
    },
  }),
}));
vi.mock("@/lib/bindings", () => ({ commands: {} }));
vi.mock("@/features/session/sessionStore", () => ({ runPromptTextInWorkstream: vi.fn() }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("@/features/workstreams/useGitActions", () => ({
  useWorkstreamGitActions: () => ({ canCommit: false, canShip: false, shipBlocked: null }),
}));

import { PromptPalette } from "./PromptPalette";
import { usePromptPaletteStore } from "./promptsStore";

const view = { item: { id: "ws1", projectPath: "/repos/app", kind: "worktree" } } as unknown as WorkstreamView;

afterEach(() => {
  cleanup();
  usePromptPaletteStore.setState({ open: false });
});

describe("PromptPalette", () => {
  it("closes on Escape when a row has focus", () => {
    usePromptPaletteStore.setState({ open: true });
    render(<PromptPalette activeWorkstream={view} />);
    screen.getByRole("option", { name: /Summarize/ }).focus();

    act(() => {
      runCommand(ESCAPE_COMMAND_ID);
    });

    expect(usePromptPaletteStore.getState().open).toBe(false);
  });
});
