// @vitest-environment jsdom
import { QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ClaudeModel, PermissionMode, Session, SessionEvent, WorkstreamView } from "@/lib/bindings";

// The app's own session store and status-bar snapshot; only the server and the heavy children are stubbed.
const h = vi.hoisted(() => {
  const ok = (data: unknown) => ({ status: "ok", data });
  const state = {
    log: [] as { seq: number; kind: string; event: unknown; createdAt: number }[],
    active: [] as unknown[],
    rows: [] as unknown[],
    running: null as string | null,
    planDefault: false,
  };
  const calls: { name: string; args: unknown[] }[] = [];
  const named: Record<string, (...args: unknown[]) => unknown> = {
    connectionStatus: () => ok({ state: "connected", transcriptSync: false }),
    getSessionLog: () => ok(state.log),
    listActiveSessions: () => ok(state.active),
    listSessions: () => ok(state.rows),
    resumeSession: () => ok({ session: state.rows[0], permissionMode: state.running }),
    startStageSession: () => ok({ session: state.rows[0], prompt: "hi" }),
  };
  const commands = new Proxy(
    {},
    {
      get: (_, name: string) =>
        async (...args: unknown[]) => {
          calls.push({ name, args });
          return (named[name] ?? (() => ok(null)))(...args);
        },
    },
  );
  const events = new Proxy({}, { get: () => ({ listen: async () => () => {} }) });
  return { ok, state, calls, commands, events };
});

vi.mock("@tauri-apps/api/core", () => ({ Channel: class { onmessage = null; } }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("@/lib/bindings", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  commands: h.commands,
  events: h.events,
}));
vi.mock("./TranscriptView", () => ({ TranscriptView: () => null }));

const LIVE: ClaudeModel[] = [
  { flag: null, resolvedModel: "claude-opus-5-5[1m]", displayName: "Default", description: null, supportedEffortLevels: ["low", "medium", "high", "xhigh"], supportsFastMode: false, disabled: false },
  { flag: "haiku", resolvedModel: "claude-haiku-5-5", displayName: "Haiku", description: null, supportedEffortLevels: ["low", "medium", "high"], supportsFastMode: false, disabled: false },
];

vi.mock("@/lib/queries", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/queries")>();
  return {
    ...actual,
    useBootstrap: () => ({
      data: {
        settings: {
          providers: {
            defaultHarness: "claude",
            // This device's own defaults, which an existing session must never show.
            claude: { defaultModel: "opus", defaultEffort: "xhigh", defaultPlanMode: h.state.planDefault },
            opencode: { selectedModels: ["opencode/beam-free"], defaultModel: "opencode/beam-free" },
            codex: { selectedModels: ["gpt-6-sol"] },
          },
          keybindings: { overrides: {} },
        },
        binaries: [{ kind: "codex", state: { state: "ok" } }],
      },
    }),
    useClaudeProbe: () => ({ data: { models: LIVE }, isPending: false, fetchStatus: "idle" }),
    useCodexProbe: () => ({ data: { models: [{ id: "gpt-6-sol", supportedEfforts: ["low", "high"] }] } }),
    useOpencodeCatalog: () => ({ data: undefined }),
    useSyncClaudeModels: () => {},
    useWorkstreamSessions: () => ({ data: h.state.rows }),
  };
});

globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;

const { queryClient } = await import("@/lib/queryClient");
const { SessionChat } = await import("./SessionChat");
const { controlsKey, useSessionControlsStore } = await import("./sessionControls");
const { dropSession, useSessionStore } = await import("./sessionStore");

const WS = "ws1";
const SID = "s1";

const VIEW = {
  item: { id: WS, projectPath: "/p", title: "ws", worktreePath: "/w", baseBranch: "main", stage: "general" },
} as unknown as WorkstreamView;

function sessionRow(over: Partial<Session>): Session {
  return {
    id: SID,
    workstreamId: WS,
    stage: "general",
    harness: "claude",
    claudeSessionId: null,
    cwd: "/w",
    state: "running",
    model: null,
    costUsd: null,
    startedAt: 1,
    endedAt: null,
    exitSummary: null,
    effort: null,
    ...over,
  };
}

const row = (seq: number, event: SessionEvent) => ({ seq, kind: event.kind, event, createdAt: 1_700_000_000_000 + seq });
const USER: SessionEvent = { kind: "userMessage", text: "hi", stage: null };
const TURN_DONE: SessionEvent = { kind: "turnDone", costUsd: null, numTurns: null, contextTokens: null };
const FAILED: SessionEvent = { kind: "exited", outcome: "spawn_failed", code: null, stderrTail: "required binary `opencode` was not found" };

const CALLBACKS = [
  "onDraftChange", "onModeChange", "onModelChange", "onEffortChange", "onPermissionModeChange", "onHarnessChange",
  "onOpencodeModelChange", "onCodexModelChange", "onCodexEffortChange", "onOpenHarnessSettings", "onSessionStarted",
  "onResetSession", "onOpenDoc", "onDetachDoc", "onSuggestDoc", "onDismissHandoffNudge", "onAttachHandoffDoc",
  "onAddFiles", "onAddScreenshots", "onAddAttachments", "onRemoveAttachment", "onClearAttachments", "onReturnQueued",
];

function chat(shell: Record<string, unknown> = {}, callbacks: Record<string, unknown> = {}) {
  const props = {
    ...Object.fromEntries(CALLBACKS.map((name) => [name, vi.fn()])),
    ...callbacks,
    view: VIEW,
    shell: { id: 1, title: "Agent", mode: "general", draft: "", docs: [], attachments: [], paneId: "p1", sessionId: SID, ...shell },
    focused: true,
    onBrowseDocs: null,
  } as unknown as Parameters<typeof SessionChat>[0];
  return (
    <QueryClientProvider client={queryClient}>
      <SessionChat {...props} />
    </QueryClientProvider>
  );
}

const renderChat = (shell: Record<string, unknown> = {}, callbacks: Record<string, unknown> = {}) =>
  render(chat(shell, callbacks));

// The page's own wiring for a reset: the tab drops its session id in the same render.
function Host({ shell }: { shell: Record<string, unknown> }) {
  const [current, setShell] = useState(shell);
  return chat(current, { onResetSession: () => setShell((s) => ({ ...s, sessionId: undefined })) });
}

const controls = () => useSessionControlsStore.getState().byShell[controlsKey(WS, 1)];
const settle = () => act(() => new Promise((r) => setTimeout(r, 0)));
const retryCalls = () =>
  h.calls.map((c) => c.name).filter((n) => /^setSession(Harness|Model|Effort)$/.test(n) || n === "resumeSession");
const argsOf = (name: string) => h.calls.find((c) => c.name === name)!.args;

beforeEach(() => {
  queryClient.clear();
  useSessionStore.setState({ sessions: {} });
  useSessionControlsStore.setState({ byShell: {} });
  h.calls.length = 0;
  h.state.running = null;
  h.state.planDefault = false;
});

afterEach(() => {
  cleanup();
  // A resume holds the session's channel, which would stop the next test's restore.
  dropSession(SID);
});

describe("a start that never launched", () => {
  beforeEach(() => {
    h.state.rows = [sessionRow({ harness: "opencode", model: "opencode/beam-free", state: "spawn_failed" })];
    h.state.active = [];
    h.state.log = [row(0, USER), row(1, FAILED)];
  });

  it("offers every provider's models, not only the one that failed", async () => {
    renderChat({ harness: "opencode" });
    await settle();

    const groups = controls()!.model!.groups;
    const claude = groups.find((g) => g.key === "claude")!.items;
    const codex = groups.find((g) => g.key === "codex")!.items;
    expect(claude.length).toBeGreaterThan(0);
    expect(claude.every((i) => !i.disabled)).toBe(true);
    expect(codex.every((i) => !i.disabled)).toBe(true);
  });

  it("retries on the Claude model picked on this tab", async () => {
    renderChat({ harness: "claude", model: "haiku" });
    await settle();
    expect(controls()).toMatchObject({ harness: "claude", model: { activeKey: "claude:haiku", label: "Haiku 5.5" } });

    fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    await settle();

    expect(retryCalls()).toEqual(["setSessionHarness", "resumeSession"]);
    expect(argsOf("setSessionHarness").slice(0, 3)).toEqual([SID, "claude", "haiku"]);
  });

  it("retries on Claude's default when the pick named no model of its own", async () => {
    renderChat({ harness: "claude" });
    await settle();

    fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    await settle();

    expect(retryCalls()).toEqual(["setSessionHarness", "resumeSession"]);
    expect(argsOf("setSessionHarness").slice(0, 3)).toEqual([SID, "claude", null]);
  });

  it("leaves the row alone while Claude is picked, so Resume writes the pick with its harness", async () => {
    const view = renderChat({ harness: "opencode" });
    await settle();

    await act(async () => controls()!.actions.pickModel("claude:haiku"));
    // What the tab holds once the pick lands.
    view.rerender(chat({ harness: "claude", model: "haiku" }));
    await act(async () => controls()!.actions.pickEffort("low"));
    await settle();

    expect(retryCalls()).toEqual([]);
  });

  it("keeps a session that ran on its provider, even when its relaunch failed", async () => {
    h.state.rows = [sessionRow({ harness: "opencode", model: "opencode/beam-free", state: "spawn_failed", claudeSessionId: "oc1" })];
    h.state.log = [
      row(0, { kind: "init", claudeSessionId: "oc1", model: "opencode/beam-free" }),
      row(1, USER),
      row(2, { kind: "turnDone", costUsd: null, numTurns: null, contextTokens: null }),
      row(3, FAILED),
    ];
    renderChat({ harness: "opencode" });
    await settle();

    const claude = controls()!.model!.groups.find((g) => g.key === "claude")!.items;
    expect(claude.every((i) => i.disabled)).toBe(true);
  });
});

describe("a session this tab did not start", () => {
  /** `mode` is what the live harness runs in; `requested` is what its start asked for, null for Auto. */
  const running = (
    model: string,
    effort: string | null,
    mode: PermissionMode | null,
    requested: PermissionMode | null = null,
  ) => {
    h.state.rows = [
      sessionRow({ harness: "claude", model, effort, claudeSessionId: "c1", requestedPermissionMode: requested }),
    ];
    h.state.active = [{ session: h.state.rows[0] }];
    h.state.log = [row(0, { kind: "init", claudeSessionId: "c1", model }), row(1, USER), row(2, TURN_DONE)];
    h.state.running = mode;
  };

  it("shows the session's model and effort, not this device's defaults", async () => {
    running("claude-haiku-5-5", "low", null);
    renderChat({ harness: "claude" });
    await settle();

    expect(controls()).toMatchObject({
      model: { activeKey: "claude:haiku", label: "Haiku 5.5" },
      effort: { activeKey: "low" },
    });
  });

  it("shows a model the catalog does not list as itself", async () => {
    running("claude-sonnet-9-1", null, null);
    renderChat({ harness: "claude" });
    await settle();

    expect(controls()!.model!.activeKey).toBe("claude:claude-sonnet-9-1");
  });

  it.each([
    ["without the catalog's [1m]", "claude-opus-5-5", "claude:opus", "Opus 5.5"],
    ["with a date the catalog lacks", "claude-haiku-5-5-20261001", "claude:haiku", "Haiku 5.5"],
  ])("names the model when init reports its id %s", async (_, id, activeKey, label) => {
    running(id, null, null);
    renderChat({ harness: "claude" });
    await settle();

    expect(controls()!.model).toMatchObject({ activeKey, label });
  });

  it("shows the mode another client started the session in, Bypass included", async () => {
    running("claude-haiku-5-5", null, "bypassPermissions", "bypassPermissions");
    renderChat({ harness: "claude" });
    await settle();

    expect(controls()!.permissionKey).toBe("bypassPermissions");
  });

  it("restarts it in that mode when the model is switched here", async () => {
    running("claude-haiku-5-5", null, "bypassPermissions", "bypassPermissions");
    renderChat({ harness: "claude" });
    await settle();

    await act(async () => controls()!.actions.pickModel("claude:opus"));
    await settle();

    expect(argsOf("restartSession")[3]).toBe("bypassPermissions");
  });

  it("keeps a tab that started its session in Auto on Auto after a reload", async () => {
    // An Auto launch runs, and reports, the stage's own mode.
    running("claude-haiku-5-5", null, "default", null);
    renderChat({ harness: "claude" });
    await settle();

    expect(controls()!.permissionKey).toBe("auto");
  });

  it("never shows this device's plan-mode default as the session's mode", async () => {
    h.state.planDefault = true;
    running("claude-haiku-5-5", null, null);
    renderChat({ harness: "claude" });
    await settle();

    expect(controls()!.permissionKey).toBe("auto");
  });
});

describe("a tab starting a session", () => {
  it("keeps the plan-mode default it launched with, so a reload still shows it", async () => {
    h.state.planDefault = true;
    h.state.rows = [sessionRow({ harness: "claude" })];
    const onPermissionModeChange = vi.fn();
    renderChat({ harness: "claude", sessionId: undefined, draft: "hi" }, { onPermissionModeChange });
    await settle();

    fireEvent.click(screen.getByRole("button", { name: "Start" }));
    await settle();

    expect(argsOf("startStageSession")[4]).toMatchObject({ permissionMode: "plan" });
    expect(onPermissionModeChange).toHaveBeenCalledWith("plan");
  });
});

describe("reopening an ended session", () => {
  const ended = (requested: PermissionMode | null) => {
    h.state.rows = [
      sessionRow({ harness: "claude", model: "claude-haiku-5-5", state: "killed", claudeSessionId: "c1", requestedPermissionMode: requested }),
    ];
    h.state.active = [];
    h.state.log = [
      row(0, { kind: "init", claudeSessionId: "c1", model: "claude-haiku-5-5" }),
      row(1, USER),
      row(2, TURN_DONE),
      row(3, { kind: "exited", outcome: "killed", code: 143, stderrTail: "" }),
    ];
  };

  it("relaunches it in the mode it was started in when the tab has no pick", async () => {
    ended("acceptEdits");
    renderChat({ harness: "claude" });
    await settle();

    expect(argsOf("resumeSession")[3]).toBe("acceptEdits");
    expect(controls()!.permissionKey).toBe("acceptEdits");
  });

  it("relaunches a session started in Auto in Auto", async () => {
    ended(null);
    renderChat({ harness: "claude" });
    await settle();

    expect(argsOf("resumeSession")[3]).toBeNull();
  });
});

describe("a tab letting go of its session", () => {
  it("stops watching it once New session leaves it", async () => {
    h.state.rows = [sessionRow({ harness: "opencode", model: "opencode/beam-free", state: "spawn_failed" })];
    h.state.active = [];
    h.state.log = [row(0, USER), row(1, FAILED)];
    render(<Host shell={{ harness: "opencode" }} />);
    await settle();
    expect(useSessionStore.getState().sessions[SID]).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "New session" }));
    await settle();

    expect(useSessionStore.getState().sessions[SID]).toBeUndefined();
  });

  it("stops watching it once Clear context ends it", async () => {
    h.state.rows = [sessionRow({ harness: "claude", model: "claude-haiku-5-5", claudeSessionId: "c1" })];
    h.state.active = [{ session: h.state.rows[0] }];
    h.state.log = [row(0, { kind: "init", claudeSessionId: "c1", model: "claude-haiku-5-5" }), row(1, USER), row(2, TURN_DONE)];
    render(<Host shell={{ harness: "claude" }} />);
    await settle();

    fireEvent.click(screen.getByRole("button", { name: "Clear context" }));
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    await settle();

    expect(useSessionStore.getState().sessions[SID]).toBeUndefined();
  });
});
