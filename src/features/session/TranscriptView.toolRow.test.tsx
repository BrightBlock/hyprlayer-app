// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createRef } from "react";

import type { TranscriptItem } from "./transcript";
import { TranscriptView } from "./TranscriptView";

const openPath = vi.hoisted(() => vi.fn(() => Promise.resolve()));
vi.mock("@tauri-apps/plugin-opener", () => ({ openPath }));

afterEach(() => {
  cleanup();
  openPath.mockClear();
});

const readTool: TranscriptItem = {
  kind: "tool",
  id: "t1",
  toolUseId: "toolu_1",
  name: "Read",
  input: JSON.stringify({ file_path: "/repo/docs/notes.md" }),
  result: { content: "ok", isError: false },
  parentId: null,
};

function renderTranscript(items: TranscriptItem[]) {
  return render(
    <TranscriptView
      ref={createRef()}
      items={items}
      busy={false}
      connecting={false}
      harnessLabel="Claude Code"
      workingSince={null}
      permission={{ onAllow: () => {}, onAllowForSession: () => {}, onDeny: () => {} }}
      prompt={{ onRewind: () => {} }}
    />,
  );
}

describe("tool row with a file card", () => {
  it("nests no button inside another button", () => {
    const { container } = renderTranscript([readTool]);
    expect(container.querySelector("button button")).toBeNull();
  });

  it("keeps the row operable as a disclosure, by pointer and by keyboard", () => {
    const { container } = renderTranscript([readTool]);
    const row = container.querySelector('[role="button"]')!;
    expect(row.getAttribute("aria-expanded")).toBe("false");

    fireEvent.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("true");

    fireEvent.keyDown(row, { key: "Enter" });
    expect(row.getAttribute("aria-expanded")).toBe("false");

    fireEvent.keyDown(row, { key: " " });
    expect(row.getAttribute("aria-expanded")).toBe("true");
  });

  it("opens the file without also toggling the row it sits in", () => {
    const { container } = renderTranscript([readTool]);
    const row = container.querySelector('[role="button"]')!;

    fireEvent.click(screen.getByTitle("Open notes.md"));

    expect(openPath).toHaveBeenCalledWith("/repo/docs/notes.md");
    expect(row.getAttribute("aria-expanded")).toBe("false");
  });
});

const asyncAgentSpawn: TranscriptItem = {
  kind: "tool",
  id: "a1",
  toolUseId: "toolu_agent",
  name: "Agent",
  input: JSON.stringify({ description: "Map core PTY runtime" }),
  result: { content: "Async agent launched successfully.", isError: false },
  parentId: null,
};

const agentChildTool: TranscriptItem = {
  kind: "tool",
  id: "a1c",
  toolUseId: "toolu_child",
  name: "Read",
  input: JSON.stringify({ file_path: "/repo/a.rs" }),
  result: { content: "ok", isError: false },
  parentId: "toolu_agent",
};

describe("a still-running background subagent row", () => {
  it("shows working… while its background task is open, though the launch ack already resolved", () => {
    renderTranscript([asyncAgentSpawn, agentChildTool]);
    expect(screen.getByText(/working…/)).toBeTruthy();
  });

  it("stops showing working… once the completion notification arrives", () => {
    renderTranscript([
      asyncAgentSpawn,
      agentChildTool,
      { kind: "taskUpdate", id: "u1", taskId: "a1", toolUseId: "toolu_agent", status: "completed" },
    ]);
    expect(screen.queryByText(/working…/)).toBeNull();
  });
});

const bash = (
  id: string,
  command: string,
  parentId: string | null = null,
  output = "ok",
): TranscriptItem => ({
  kind: "tool",
  id,
  toolUseId: `toolu_${id}`,
  name: "Bash",
  input: JSON.stringify({ command }),
  result: { content: output, isError: false },
  parentId,
});

describe("consecutive tool calls", () => {
  it("each show as their own row in the main transcript", () => {
    renderTranscript([bash("b1", "git status"), bash("b2", "git diff"), bash("b3", "pnpm test")]);
    expect(screen.queryByText(/tool calls/)).toBeNull();
    for (const command of ["git status", "git diff", "pnpm test"]) {
      expect(screen.getByText(command)).toBeTruthy();
    }
  });

  it("read as Name(args) with a summarized result, the way Claude Code prints them", () => {
    const { container } = renderTranscript([bash("b1", "pnpm test", null, "one\ntwo\nthree")]);
    expect(container.textContent).toContain("Bash(pnpm test)");
    expect(screen.getByText("one")).toBeTruthy();
    expect(screen.getByText("… +2 lines")).toBeTruthy();
  });

  it("still fold together inside a subagent", () => {
    renderTranscript([
      asyncAgentSpawn,
      bash("c1", "git status", "toolu_agent"),
      bash("c2", "git diff", "toolu_agent"),
    ]);
    fireEvent.click(screen.getByText("Subagent"));
    expect(screen.getByText("2 tool calls")).toBeTruthy();
    expect(screen.queryByText("git status")).toBeNull();
  });
});

const askCard = (
  decision: "expired" | null,
  toolName = "Bash",
): Extract<TranscriptItem, { kind: "permission" }> => ({
  kind: "permission",
  id: `p-${toolName}`,
  requestId: "req-1",
  toolName,
  input: JSON.stringify({ command: "git status --short" }),
  decision,
});

describe("a permission card", () => {
  it("offers Allow and Deny while its ask is open", () => {
    renderTranscript([askCard(null)]);
    expect(screen.getByRole("button", { name: "Allow once" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Deny" })).toBeTruthy();
  });

  it("reads as expired, with no actions, once its ask died with the session", () => {
    renderTranscript([askCard("expired")]);
    expect(screen.queryByRole("button", { name: /Allow|Deny/ })).toBeNull();
    expect(screen.queryByText("Permission required")).toBeNull();
    expect(screen.getByText(/expired/)).toBeTruthy();
  });

  it("shows an expired question the same way, naming what was asked", () => {
    renderTranscript([
      {
        ...askCard("expired", "AskUserQuestion"),
        input: JSON.stringify({ questions: [{ question: "Push the branch too?" }] }),
      },
    ]);
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getByText(/Push the branch too\?/)).toBeTruthy();
    expect(screen.getByText(/expired/)).toBeTruthy();
  });
});
