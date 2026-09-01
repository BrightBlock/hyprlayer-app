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
