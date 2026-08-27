// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AgentFile } from "@/lib/bindings";

import { AgentSidebar, type AgentSidebarProps } from "./AgentSidebar";
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

function sidebar(rows: AgentRow[] = ROWS, overrides: Partial<AgentSidebarProps> = {}) {
  const onFolderMenu = vi.fn();
  const onMenu = vi.fn();
  render(
    <AgentSidebar
      width={288}
      collapsed={false}
      section="agents"
      onSection={vi.fn()}
      selectedAgent={null}
      onBack={vi.fn()}
      editorTab="setup"
      onEditorTab={vi.fn()}
      rows={rows}
      agentsCount={rows.filter((r) => r.kind === "agent").length}
      library={undefined}
      projects={[]}
      selectedProject={null}
      onSelectProject={vi.fn()}
      search=""
      onSearch={vi.fn()}
      onToggleFolder={vi.fn()}
      onOpen={vi.fn()}
      onMenu={onMenu}
      onFolderMenu={onFolderMenu}
      dotColorOf={() => "red"}
      loading={false}
      onCreate={vi.fn()}
      onNewFolder={vi.fn()}
      groupUncategorized
      onGroupUncategorized={vi.fn()}
      bundle={{
        state: { kind: "unknown", detail: "" },
        updating: false,
        checking: false,
        onRefresh: vi.fn(),
        onUpdate: vi.fn(),
      }}
      {...overrides}
    />,
  );
  return { onFolderMenu, onMenu };
}

describe("AgentSidebar bundle chip", () => {
  const chip = (state: AgentSidebarProps["bundle"]["state"]) =>
    sidebar(undefined, {
      bundle: {
        state,
        updating: false,
        checking: false,
        onRefresh: vi.fn(),
        onUpdate: vi.fn(),
      },
    });

  it("says the bundle is up to date on a CLI that installs a live one", () => {
    chip({ kind: "upToDate", label: "1.6.0", lastCheckedMs: null, frozen: false });
    expect(screen.getByText("Up to date")).toBeTruthy();
    expect(screen.queryByText("CLI out of date")).toBeNull();
  });

  it("blames the CLI, not the bundle, when the tree it tracks is frozen", () => {
    chip({ kind: "upToDate", label: "9a2a33c", lastCheckedMs: null, frozen: true });
    expect(screen.getByText("CLI out of date")).toBeTruthy();
    expect(screen.queryByText("Up to date")).toBeNull();
  });
});

describe("AgentSidebar folder rows", () => {
  it("reports a right-click on a folder header, so an agent can be made inside it", () => {
    const { onFolderMenu } = sidebar();
    fireEvent.contextMenu(screen.getByText("Review"), { clientX: 120, clientY: 240 });
    expect(onFolderMenu).toHaveBeenCalledWith("Review", { x: 120, y: 240 });
  });

  it("keeps the folder's own menu off the agent menu", () => {
    const { onMenu } = sidebar();
    fireEvent.contextMenu(screen.getByText("Review"));
    expect(onMenu).not.toHaveBeenCalled();
  });

  it("draws an empty folder that agentRows kept, with its zero count", () => {
    sidebar([{ kind: "group", folder: "Brand new", count: 0, open: true }]);
    const row = screen.getByText("Brand new").closest("button");
    expect(row).toBeTruthy();
    expect(within(row as HTMLElement).getByText("0")).toBeTruthy();
  });
});
