// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AgentFile } from "@/lib/bindings";
import { MODELS } from "@/lib/domain/models";

import { AgentList, type AgentListProps } from "./AgentList";
import type { AgentRow } from "./agents";

afterEach(cleanup);

const AGENT: AgentFile = {
  path: "/a/scout.md",
  source: "user",
  plugin: null,
  managed: false,
  name: "scout",
  description: "",
  tools: [],
  skills: [],
  model: null,
  color: null,
  memory: null,
  category: "Review",
  prompt: "",
  updatedAt: null,
  shippedPath: null,
  modified: false,
};

const ROWS: AgentRow[] = [
  { kind: "group", folder: "Review", count: 1, open: true },
  { kind: "agent", agent: AGENT, indent: 14 },
];

function list(rows: AgentRow[] = ROWS, overrides: Partial<AgentListProps> = {}) {
  const onFolderMenu = vi.fn();
  const onMenu = vi.fn();
  render(
    <AgentList
      section="agents"
      folder={null}
      rows={rows}
      flat={[]}
      systemInFolder={[]}
      total={rows.filter((r) => r.kind === "agent").length}
      search=""
      onSearch={vi.fn()}
      selectedPath={null}
      library={undefined}
      catalog={MODELS}
      loading={false}
      onOpen={vi.fn()}
      onMenu={onMenu}
      onFolderMenu={onFolderMenu}
      onToggleFolder={vi.fn()}
      onNewFolder={vi.fn()}
      dotColorOf={() => "red"}
      {...overrides}
    />,
  );
  return { onFolderMenu, onMenu };
}

describe("AgentList folder rows", () => {
  it("reports a right-click on a folder header, so an agent can be made inside it", () => {
    const { onFolderMenu } = list();
    fireEvent.contextMenu(screen.getByText("Review"), { clientX: 120, clientY: 240 });
    expect(onFolderMenu).toHaveBeenCalledWith("Review", { x: 120, y: 240 });
  });

  it("keeps the folder's own menu off the agent menu", () => {
    const { onMenu } = list();
    fireEvent.contextMenu(screen.getByText("Review"));
    expect(onMenu).not.toHaveBeenCalled();
  });

  it("draws an empty folder that agentRows kept, with its zero count", () => {
    list([{ kind: "group", folder: "Brand new", count: 0, open: true }]);
    const row = screen.getByText("Brand new").closest("button");
    expect(row).toBeTruthy();
    expect(within(row as HTMLElement).getByText("0")).toBeTruthy();
  });
});
