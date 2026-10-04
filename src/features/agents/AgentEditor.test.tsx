// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AgentFile } from "@/lib/bindings";

import { AgentEditor } from "./AgentEditor";
import { DEFAULT_OVERLAY } from "./agents";

vi.mock("@/lib/account", () => ({ useSignedIn: () => false }));

afterEach(cleanup);

const agent = (managed: boolean): AgentFile => ({
  path: "/home/u/.claude/agents/cartographer.md",
  source: "user",
  plugin: null,
  managed,
  name: "cartographer",
  description: "Maps a codebase",
  tools: ["Read"],
  skills: [],
  model: null,
  color: null,
  memory: null,
  category: null,
  prompt: "Map the codebase.",
  updatedAt: null,
  shippedPath: null,
  modified: false,
});

function renderEditor(draft: AgentFile) {
  render(
    <QueryClientProvider client={new QueryClient()}>
      <AgentEditor
        draft={draft}
        overlay={DEFAULT_OVERLAY}
        isDefault={false}
        patch={vi.fn()}
        patchOverlay={vi.fn()}
        onDefault={vi.fn()}
        dirty={true}
        tab="setup"
        onTab={vi.fn()}
        onBack={vi.fn()}
        onSave={vi.fn()}
        onStatus={vi.fn()}
        onReset={vi.fn()}
        onExport={vi.fn()}
        onCopyPath={vi.fn()}
        saving={false}
        projectName={null}
        folders={[]}
        contextFiles={[]}
        mcpServers={[]}
        skills={[]}
        peers={[]}
      />
    </QueryClientProvider>,
  );
}

describe("AgentEditor managed agents", () => {
  it("renders a store-linked agent as read-only with no save or reset actions", () => {
    renderEditor(agent(true));

    expect(screen.getByText("View Managed Agent")).toBeTruthy();
    expect(screen.getByText("Read-only")).toBeTruthy();
    expect(screen.getByText(/manages this linked agent/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
    expect(screen.queryByRole("button", { name: "More actions" })).toBeNull();
    expect(screen.getByDisplayValue("cartographer").matches(":disabled")).toBe(true);
    expect(screen.getByDisplayValue("Maps a codebase").matches(":disabled")).toBe(true);
  });

  it("keeps a regular personal agent editable", () => {
    renderEditor(agent(false));

    expect(screen.getByText("Edit Agent")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Save" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "More actions" })).toBeTruthy();
    expect((screen.getByDisplayValue("cartographer") as HTMLInputElement).disabled).toBe(false);
  });
});
