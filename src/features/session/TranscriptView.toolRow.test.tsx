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

/** A `Read` call: `ToolArg` renders these as a `FileCard`, whose filename is a
 *  button. Any file-targeting tool (Read/Write/Edit/…) reaches the same card. */
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
    // The defect this guards: the row used to be a <button> and the card's
    // filename is one, which React rejects as invalid HTML.
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
    // The user-visible half of the same defect: the inner click bubbled out to
    // the row, so opening a file expanded a disclosure nobody asked for.
    const { container } = renderTranscript([readTool]);
    const row = container.querySelector('[role="button"]')!;

    fireEvent.click(screen.getByTitle("Open notes.md"));

    expect(openPath).toHaveBeenCalledWith("/repo/docs/notes.md");
    expect(row.getAttribute("aria-expanded")).toBe("false");
  });
});
