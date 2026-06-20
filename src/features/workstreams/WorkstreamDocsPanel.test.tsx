// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi, beforeEach } from "vitest";

import { WorkstreamDocsPanel, roleForDoc } from "./WorkstreamDocsPanel";
import { commands, type DocMeta } from "../../lib/bindings";

vi.mock("../../lib/bindings", () => ({
  commands: {
    queryDocs: vi.fn(),
  },
}));

// No vitest globals -> RTL can't register its own auto-cleanup; without this,
// a previous test's mounted copy steals the next test's queries and clicks.
afterEach(cleanup);

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

function makeDoc(overrides: { id: string; title: string; type: string }): DocMeta {
  return {
    id: overrides.id,
    relPath: overrides.id.replace(/^\//, ""),
    project: "myproj",
    area: { kind: "shared" },
    scope: "shared",
    profile: null,
    backend: "git",
    fm: {
      title: overrides.title,
      type: overrides.type,
      date: "2026-06-12",
      status: "draft",
      ticket: null,
      project: "myproj",
      scope: "shared",
      tags: [],
      author: "jt",
      related: [],
      extra: {},
    },
    fmWarnings: [],
    mtimeMs: 0,
    sizeBytes: 0,
  } as DocMeta;
}

const RESEARCH_DOC = makeDoc({ id: "/t/research/a.md", title: "Auth research", type: "research" });
const NOTE_DOC = makeDoc({ id: "/t/notes/b.md", title: "Random note", type: "note" });

function renderPanel(attachedPaths: string[] = [RESEARCH_DOC.id]) {
  const onAttach = vi.fn();
  const onDetach = vi.fn();
  const onOpenDoc = vi.fn();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <WorkstreamDocsPanel
        thoughtsProject="myproj"
        targetTitle="Agent 2"
        attachedPaths={attachedPaths}
        onAttach={onAttach}
        onDetach={onDetach}
        onOpenDoc={onOpenDoc}
      />
    </QueryClientProvider>,
  );
  return { onAttach, onDetach, onOpenDoc };
}

describe("roleForDoc", () => {
  it("maps research/plan types to roles, everything else to other", () => {
    expect(roleForDoc(RESEARCH_DOC)).toBe("research");
    expect(roleForDoc(makeDoc({ id: "/x", title: "p", type: "plan" }))).toBe("plan");
    expect(roleForDoc(NOTE_DOC)).toBe("other");
  });
});

describe("WorkstreamDocsPanel", () => {
  beforeEach(() => {
    vi.mocked(commands.queryDocs).mockReturnValue(
      ok({ total: 2, items: [RESEARCH_DOC, NOTE_DOC] }) as never,
    );
  });

  it("scopes the query to the project, names the target tab, shows attach state", async () => {
    renderPanel();
    expect(await screen.findByText("Auth research")).toBeTruthy();
    expect(screen.getByText("Random note")).toBeTruthy();
    expect(screen.getByText("Agent 2")).toBeTruthy();
    // Already-attached doc offers Detach; the other offers Attach.
    expect(screen.getByText("Detach")).toBeTruthy();
    expect(screen.getByText("Attach")).toBeTruthy();
    const query = vi.mocked(commands.queryDocs).mock.calls[0]![0];
    expect(query.projects).toEqual(["myproj"]);
  });

  it("attaches an unattached doc with its derived role", async () => {
    const { onAttach } = renderPanel();
    fireEvent.click(await screen.findByText("Attach"));
    expect(onAttach).toHaveBeenCalledWith(NOTE_DOC.id, "other");
  });

  it("detaches an attached doc", async () => {
    const { onDetach } = renderPanel();
    fireEvent.click(await screen.findByText("Detach"));
    expect(onDetach).toHaveBeenCalledWith(RESEARCH_DOC.id);
  });

  it("opens a doc from its row", async () => {
    const { onOpenDoc } = renderPanel();
    fireEvent.click(await screen.findByText("Random note"));
    expect(onOpenDoc).toHaveBeenCalledWith(NOTE_DOC.id);
  });
});
