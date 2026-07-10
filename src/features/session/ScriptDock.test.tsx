// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// `scriptsStore` persists through zustand's `persist`, which needs a Storage at
// module-eval time; vitest's jsdom doesn't expose one. Hoisted so it lands before
// the store module below is evaluated.
vi.hoisted(() => {
  const mem = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => mem.get(k) ?? null,
    setItem: (k: string, v: string) => void mem.set(k, v),
    removeItem: (k: string) => void mem.delete(k),
    clear: () => mem.clear(),
    key: (i: number) => [...mem.keys()][i] ?? null,
    get length() {
      return mem.size;
    },
  });
});

import { ScriptDock } from "./ScriptDock";
import { requestRun, useScriptsStore } from "./scriptsStore";

// Mock the data hooks so the dock renders off plain values — no react-query, no IPC.
const q = vi.hoisted(() => ({
  useBootstrap: vi.fn(),
  useScriptOutput: vi.fn(),
  useScriptRuntime: vi.fn(),
  useScripts: vi.fn(),
  useShellName: vi.fn(),
  forgetScriptOutput: vi.fn(),
}));
vi.mock("@/lib/queries", () => q);

const b = vi.hoisted(() => ({
  commands: { scriptStop: vi.fn(), scriptClearOutput: vi.fn(), terminalClose: vi.fn() },
  events: { scriptRunChanged: { listen: vi.fn() } },
}));
vi.mock("@/lib/bindings", () => b);

// Stub the terminal-backed bodies: this suite is about which body the dock picks
// and whether a run is spawned at all — not about xterm.
vi.mock("./ScriptRunView", () => ({ ScriptRunView: () => <div data-testid="run-view" /> }));
vi.mock("./ScriptOutputView", () => ({
  ScriptOutputView: ({ output }: { output: string }) => <div data-testid="output-view">{output}</div>,
}));
vi.mock("./TerminalTab", () => ({ TerminalTab: () => <div data-testid="terminal-tab" /> }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

// The tab strip observes its own width; jsdom has no ResizeObserver.
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;

const WS = "ws1";
const scripts = (run: string) => ({ data: { setup: "", run, archive: "" }, isPending: false });

beforeEach(() => {
  vi.clearAllMocks();
  useScriptsStore.setState({ dock: {}, running: {}, pendingRun: {}, activeWorkstream: null });
  q.useBootstrap.mockReturnValue({ data: { settings: { keybindings: { overrides: {} } } } });
  q.useScriptOutput.mockReturnValue({ data: null, isLoading: false });
  q.useScriptRuntime.mockReturnValue({ data: { runTerminalId: null, port: null, statuses: [] } });
  q.useShellName.mockReturnValue({ data: "zsh" });
  q.useScripts.mockReturnValue(scripts(""));
});
afterEach(cleanup);

const renderDock = () =>
  render(<ScriptDock workstreamId={WS} projectPath="/p" onEditScripts={vi.fn()} />);

const pendingRun = () => useScriptsStore.getState().pendingRun[WS];

describe("ScriptDock ⌘R", () => {
  it("does not start a run when no Run script is configured", () => {
    q.useScripts.mockReturnValue(scripts(""));
    requestRun(WS);
    renderDock();

    // No run view spawned; the tab explains why nothing happened.
    expect(screen.queryByTestId("run-view")).toBeNull();
    expect(screen.getByText(/No Run script configured/i)).toBeTruthy();
    // The one-shot flag is still consumed, so it can't fire on a later mount.
    expect(pendingRun()).toBeUndefined();
  });

  it("starts the run when one is configured", () => {
    q.useScripts.mockReturnValue(scripts("pnpm dev"));
    requestRun(WS);
    renderDock();

    expect(screen.getByTestId("run-view")).toBeTruthy();
    expect(pendingRun()).toBeUndefined();
  });

  it("waits for the project's scripts to load before deciding", () => {
    // An unresolved query reads as "no run script" — deciding now would swallow
    // a legitimate ⌘R against a project that does have one.
    q.useScripts.mockReturnValue({ data: undefined, isPending: true });
    requestRun(WS);
    const { rerender } = renderDock();

    expect(screen.queryByTestId("run-view")).toBeNull();
    expect(pendingRun()).toBe(true); // held, not consumed

    q.useScripts.mockReturnValue(scripts("pnpm dev"));
    rerender(<ScriptDock workstreamId={WS} projectPath="/p" onEditScripts={vi.fn()} />);

    expect(screen.getByTestId("run-view")).toBeTruthy();
    expect(pendingRun()).toBeUndefined();
  });
});

describe("ScriptDock clear output", () => {
  it("is disabled when there is nothing to clear", () => {
    q.useScripts.mockReturnValue(scripts("pnpm dev"));
    renderDock();

    expect(screen.getByLabelText("Clear output")).toHaveProperty("disabled", true);
  });

  it("forgets a finished run's captured output and falls back to the idle state", () => {
    q.useScripts.mockReturnValue(scripts("pnpm dev"));
    q.useScriptOutput.mockReturnValue({ data: "old log", isLoading: false });
    renderDock();

    expect(screen.getByTestId("output-view").textContent).toBe("old log");

    fireEvent.click(screen.getByLabelText("Clear output"));

    // No live PTY to wipe (the run is over), so no terminal id is passed.
    expect(b.commands.scriptClearOutput).toHaveBeenCalledWith(WS, "run", null);
    expect(q.forgetScriptOutput).toHaveBeenCalledWith(WS, "run");
  });

  it("passes the live PTY id so a re-attach can't replay cleared output", () => {
    q.useScripts.mockReturnValue(scripts("pnpm dev"));
    q.useScriptRuntime.mockReturnValue({
      data: { runTerminalId: "pty-7", port: null, statuses: [] },
    });
    renderDock();

    expect(screen.getByTestId("run-view")).toBeTruthy();
    fireEvent.click(screen.getByLabelText("Clear output"));

    expect(b.commands.scriptClearOutput).toHaveBeenCalledWith(WS, "run", "pty-7");
  });
});
