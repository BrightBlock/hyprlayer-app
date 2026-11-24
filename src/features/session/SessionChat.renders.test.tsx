// @vitest-environment jsdom
import { QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Goal, SessionEvent, WorkstreamView } from "@/lib/bindings";

import type { TranscriptItem } from "./transcript";

// The app's own stores; only the server and the transcript are stubbed.
const h = vi.hoisted(() => {
  const ok = (data: unknown) => ({ status: "ok", data });
  const state = { goals: [] as unknown[] };
  const named: Record<string, () => unknown> = {
    connectionStatus: () => ok({ state: "connected", transcriptSync: false }),
    listActiveSessions: () => ok([]),
    listSessions: () => ok([]),
    listGoals: () => ok(state.goals),
  };
  const commands = new Proxy(
    {},
    { get: (_, name: string) => async () => (named[name] ?? (() => ok(null)))() },
  );
  const events = new Proxy({}, { get: () => ({ listen: async () => () => {} }) });
  return { state, commands, events };
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

globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;

const { queryClient } = await import("@/lib/queryClient");
const { usePlanAside } = await import("@/features/plan/usePlanAside");
const { useGoalAside } = await import("./goalAsideState");
const { SessionChat } = await import("./SessionChat");
const { useComposerDraftsStore } = await import("./composerDrafts");
const { dropSession, useSessionStore } = await import("./sessionStore");
const { initialSessionView, reduceEvent } = await import("./transcript");
const { leaf } = await import("./paneTree");
const { updateWorkstreamTabs, useWorkstreamTabsStore } = await import("./workstreamTabs");

const WS = "ws1";
const SID = "s1";
const PLAN = "/t/repos/app/shared/plans/2026-09-24-og.md";

const GOAL: Goal = {
  id: "g1",
  sessionId: SID,
  workstreamId: WS,
  condition: "the plan is approved",
  workPrompt: "",
  source: "composer",
  sourceRef: null,
  sourcePhase: null,
  checks: [],
  limits: { maxIterations: 5, maxCostUsd: null, maxTokens: null, maxActiveSecs: 3600 },
  status: "running",
  step: "working",
  pauseReason: null,
  endReason: null,
  iteration: 1,
  spentCostUsd: null,
  spentTokens: 0,
  activeMs: 0,
  lastOutcome: null,
  lastReason: null,
  author: null,
  startedAt: 1,
  updatedAt: 1,
  endedAt: null,
};

const VIEW = {
  item: { id: WS, projectPath: "/p", title: "ws", worktreePath: "/w", baseBranch: "main", stage: "general" },
} as unknown as WorkstreamView;

const CALLBACKS = Object.fromEntries(
  [
    "onModeChange", "onModelChange", "onEffortChange", "onPermissionModeChange", "onHarnessChange",
    "onOpencodeModelChange", "onCodexModelChange", "onCodexEffortChange", "onOpenHarnessSettings",
    "onSessionStarted", "onResetSession", "onOpenDoc", "onDetachDoc", "onSuggestDoc", "onDismissHandoffNudge",
    "onAttachHandoffDoc", "onAddFiles", "onAddScreenshots", "onAddAttachments", "onRemoveAttachment",
    "onClearAttachments", "onReturnQueued",
  ].map((name) => [name, vi.fn()]),
);

let pageRenders = 0;

// Stands in for ProjectPage: it reads every workstream's tabs and the focused session's asides,
// and renders that tab's composer.
function Page() {
  pageRenders += 1;
  const shell = useWorkstreamTabsStore((s) => s.byWs)[WS]!.shells[0];
  const plan = usePlanAside(shell.sessionId, false);
  const goal = useGoalAside(shell.sessionId, WS, plan.available, false);
  const props = { ...CALLBACKS, view: VIEW, shell, focused: true, onBrowseDocs: null };
  const aside = `${plan.available ? "plan" : "no plan"} · ${goal.available ? "goal" : "no goal"}`;
  return (
    <>
      <output aria-label="aside">{aside}</output>
      <SessionChat {...(props as unknown as Parameters<typeof SessionChat>[0])} />
    </>
  );
}

const renderPage = () =>
  render(
    <QueryClientProvider client={queryClient}>
      <Page />
    </QueryClientProvider>,
  );

const settle = () => act(() => new Promise((r) => setTimeout(r, 0)));

function openTab(sessionId?: string) {
  const shell = { id: 1, title: "Agent", mode: "general" as const, docs: [], attachments: [], paneId: "p1", sessionId };
  useWorkstreamTabsStore.setState({
    byWs: { [WS]: { shells: [shell], layout: leaf("p1"), panes: { p1: { activeId: 1 } }, activePaneId: "p1" } },
  });
}

function startSession() {
  const user: TranscriptItem = {
    kind: "user",
    id: "u",
    text: "/create_plan",
    stage: null,
    attachments: [],
    createdAt: null,
    promptName: null,
  };
  useSessionStore.setState({
    sessions: {
      [SID]: {
        ...initialSessionView("running"),
        items: [user],
        workstreamId: WS,
        stage: "plan",
        autoAllow: [],
        live: true,
        permissionMode: null,
        queued: [],
        workingSince: null,
        taskStartedAt: {},
      },
    },
  });
}

function stream(event: SessionEvent) {
  act(() =>
    useSessionStore.setState((s) => {
      const entry = s.sessions[SID]!;
      return { sessions: { ...s.sessions, [SID]: { ...entry, ...reduceEvent(entry, event) } } };
    }),
  );
}

const composer = () => screen.getByRole("textbox") as HTMLTextAreaElement;

beforeEach(() => {
  queryClient.clear();
  useSessionStore.setState({ sessions: {} });
  useComposerDraftsStore.setState({ byWs: {} });
  h.state.goals = [];
  pageRenders = 0;
});

afterEach(() => {
  cleanup();
  dropSession(SID);
});

describe("the page around a composer", () => {
  it("does not re-render while the user types, and the text stays with the tab", async () => {
    openTab();
    const { unmount } = renderPage();
    await settle();
    const before = pageRenders;

    for (const value of ["h", "he", "hel", "hello"]) fireEvent.change(composer(), { target: { value } });

    expect(composer().value).toBe("hello");
    expect(pageRenders).toBe(before);

    unmount();
    renderPage();
    expect(composer().value).toBe("hello");
  });

  it("still re-renders when the tabs themselves change", async () => {
    openTab();
    renderPage();
    await settle();
    const before = pageRenders;

    act(() =>
      updateWorkstreamTabs((all) => ({
        ...all,
        [WS]: { ...all[WS], shells: all[WS].shells.map((s) => ({ ...s, title: "Renamed" })) },
      })),
    );

    expect(pageRenders).toBeGreaterThan(before);
  });

  it("does not re-render for streamed events, only when they change its plan or goal aside", async () => {
    h.state.goals = [GOAL];
    startSession();
    openTab(SID);
    renderPage();
    await waitFor(() => expect(screen.getByLabelText("aside").textContent).toBe("no plan · goal"));
    await waitFor(() => expect(queryClient.isFetching()).toBe(0));
    await settle();
    const before = pageRenders;

    stream({ kind: "partialText", text: "Drafting", parentId: null });
    stream({ kind: "partialText", text: " the plan", parentId: null });
    stream({ kind: "assistantText", text: "Drafting the plan", parentId: null });
    const input = JSON.stringify({ file_path: PLAN, content: "" });
    stream({ kind: "toolUse", id: "tu-w", name: "Write", input, parentId: null });
    expect(pageRenders).toBe(before);

    stream({ kind: "toolResult", toolUseId: "tu-w", content: "ok", isError: false, parentId: null });
    expect(screen.getByLabelText("aside").textContent).toBe("plan · goal");
    expect(pageRenders).toBeGreaterThan(before);
  });
});
