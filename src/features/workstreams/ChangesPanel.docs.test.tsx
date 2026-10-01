// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SessionEvent } from "@/lib/bindings";
import { useUiStore } from "@/lib/uiStore";
import { ChangesPanel } from "./ChangesPanel";

const q = vi.hoisted(() => ({
  useWorktreeGitStatus: vi.fn(),
  useWorktreeDir: vi.fn(),
  useStageFiles: vi.fn(),
  useUnstageFiles: vi.fn(),
  useDiscardFiles: vi.fn(),
  useWorkstreamCommits: vi.fn(),
  useCommitFiles: vi.fn(),
  useProjectMemory: vi.fn(),
}));
vi.mock("@/lib/queries", () => q);
vi.mock("@/lib/queryClient", () => ({ queryClient: { invalidateQueries: vi.fn() } }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

const h = vi.hoisted(() => {
  class FakeChannel {
    onmessage: ((event: unknown) => void) | null = null;
  }
  const channels = new Map<string, FakeChannel>();
  let nextId = 0;
  const startStageSession = vi.fn(async (...args: unknown[]) => {
    const id = `s${(nextId += 1)}`;
    channels.set(id, args[args.length - 1] as FakeChannel);
    return { status: "ok", data: { session: { id, model: "m", claudeSessionId: `c-${id}` }, prompt: "" } };
  });
  return { FakeChannel, channels, startStageSession };
});
vi.mock("@tauri-apps/api/core", () => ({ Channel: h.FakeChannel }));
vi.mock("@/lib/bindings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/bindings")>();
  return { ...actual, commands: { ...actual.commands, startStageSession: h.startStageSession } };
});

const { startSession, useSessionStore } = await import("@/features/session/sessionStore");

function emitToWorkstream(workstreamId: string, tool: { name: string; input: string }) {
  return startSession({ workstreamId, stage: "general", input: "go" }).then((session) => {
    const channel = h.channels.get(session.id);
    const toolUse: SessionEvent = {
      kind: "toolUse",
      id: "tu1",
      name: tool.name,
      input: tool.input,
      parentId: null,
    };
    channel?.onmessage?.({ seq: null, rev: null, author: null, event: toolUse });
  });
}

function setup() {
  q.useWorktreeGitStatus.mockReturnValue({
    data: { staged: [], unstaged: [] },
    isLoading: false,
    isError: false,
    isFetching: false,
    error: null,
    refetch: vi.fn(),
  });
  q.useWorktreeDir.mockReturnValue({ data: [], isLoading: false, isError: false, error: null });
  const mutation = () => ({ mutate: vi.fn(), isPending: false });
  q.useStageFiles.mockReturnValue(mutation());
  q.useUnstageFiles.mockReturnValue(mutation());
  q.useDiscardFiles.mockReturnValue(mutation());
  q.useWorkstreamCommits.mockReturnValue({
    data: [],
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  });
  q.useProjectMemory.mockReturnValue({
    data: { dir: "/c/projects/-r/memory", keyCheck: "verified", files: [] },
    isLoading: false,
    isError: false,
    error: null,
  });
}

beforeEach(() => {
  useSessionStore.setState({ sessions: {} });
  useUiStore.setState({ docsCollapsed: false });
  h.channels.clear();
  h.startStageSession.mockClear();
});

afterEach(cleanup);

describe("ChangesPanel Documents section, real attribution", () => {
  const PLAN = "/t/shared/plans/2026-08-21-scope.md";

  it("lists a plan the workstream's own agent wrote", async () => {
    await emitToWorkstream("wsA", { name: "Write", input: JSON.stringify({ file_path: PLAN }) });
    setup();
    render(
      <ChangesPanel workstreamId="wsA" onOpenDoc={vi.fn()} onOpenDiff={vi.fn()} onOpenFile={vi.fn()} baseBranch="main" />,
    );
    expect(screen.getByText("Documents")).toBeDefined();
    expect(screen.getByText("scope")).toBeDefined();
  });

  it("renders no Documents section for a workstream that only read the same plan", async () => {
    await emitToWorkstream("wsA", { name: "Write", input: JSON.stringify({ file_path: PLAN }) });
    await emitToWorkstream("wsB", { name: "Bash", input: JSON.stringify({ command: `cat ${PLAN}` }) });
    setup();
    render(
      <ChangesPanel workstreamId="wsB" onOpenDoc={vi.fn()} onOpenDiff={vi.fn()} onOpenFile={vi.fn()} baseBranch="main" />,
    );
    expect(screen.queryByText("Documents")).toBeNull();
    expect(screen.queryByText("scope")).toBeNull();
  });

  it("collapses the list to its header, leaving the section title visible", async () => {
    await emitToWorkstream("wsA", { name: "Write", input: JSON.stringify({ file_path: PLAN }) });
    setup();
    render(
      <ChangesPanel workstreamId="wsA" onOpenDoc={vi.fn()} onOpenDiff={vi.fn()} onOpenFile={vi.fn()} baseBranch="main" />,
    );
    fireEvent.click(screen.getByLabelText("Hide documents, 1"));
    expect(screen.queryByText("scope")).toBeNull();
    expect(screen.getByText("Documents")).toBeDefined();
    expect(screen.getByLabelText("Show documents, 1")).toBeDefined();
    expect(useUiStore.getState().docsCollapsed).toBe(true);
  });
});

describe("ChangesPanel memory view", () => {
  const DIR = "/c/projects/-r/memory";
  const file = (name: string) => ({ name, path: `${DIR}/${name}`, sizeBytes: 10, mtimeMs: 0 });

  function renderWithMemory(keyCheck: string, files: ReturnType<typeof file>[]) {
    setup();
    q.useProjectMemory.mockReturnValue({
      data: { dir: DIR, keyCheck, files },
      isLoading: false,
      isError: false,
      error: null,
    });
    const onOpenDoc = vi.fn();
    render(
      <ChangesPanel workstreamId="wsA" onOpenDoc={onOpenDoc} onOpenDiff={vi.fn()} onOpenFile={vi.fn()} baseBranch="main" />,
    );
    return onOpenDoc;
  }

  it("opens a memory file in the document preview", () => {
    const onOpenDoc = renderWithMemory("verified", [file("MEMORY.md"), file("topic.md")]);
    fireEvent.click(screen.getByRole("button", { name: "Project memory" }));
    expect(screen.getByText("2")).toBeDefined();
    fireEvent.click(screen.getByText("topic.md"));
    expect(onOpenDoc).toHaveBeenCalledWith(`${DIR}/topic.md`, "topic.md");
  });

  it("names the directory Claude Code writes to when there is no memory yet", () => {
    renderWithMemory("verified", []);
    fireEvent.click(screen.getByRole("button", { name: "Project memory" }));
    expect(screen.getByText(DIR)).toBeDefined();
  });

  it("warns instead of listing files when the key check fails", () => {
    renderWithMemory("mismatch", [file("MEMORY.md")]);
    fireEvent.click(screen.getByRole("button", { name: "Project memory" }));
    expect(screen.getByText("!")).toBeDefined();
    expect(screen.getByText(/somewhere hyprlayer does not compute/)).toBeDefined();
    expect(screen.queryByText("MEMORY.md")).toBeNull();
  });
});
