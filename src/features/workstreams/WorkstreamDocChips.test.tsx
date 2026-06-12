// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi, beforeEach } from "vitest";

// No vitest globals -> RTL can't register its own auto-cleanup; without this,
// a previous test's mounted copy steals the next test's queries and clicks.
afterEach(cleanup);

import { WorkstreamDocChips, chipLabel } from "./WorkstreamDocChips";
import { commands } from "../../lib/bindings";

vi.mock("../../lib/bindings", () => ({
  commands: {
    listWorkstreamDocs: vi.fn(),
    detachWorkstreamDoc: vi.fn(),
  },
}));

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const RESEARCH = {
  workstreamId: "ws1",
  docPath: "/t/repos/p/shared/research/2026-06-12-auth-research.md",
  role: "research",
};
const PLAN = {
  workstreamId: "ws1",
  docPath: "/t/repos/p/shared/plans/2026-06-12-auth-plan.md",
  role: "plan",
};

function renderChips(onOpenDoc = vi.fn(), onBrowse = vi.fn()) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <WorkstreamDocChips workstreamId="ws1" onOpenDoc={onOpenDoc} onBrowse={onBrowse} />
    </QueryClientProvider>,
  );
  return { onOpenDoc, onBrowse };
}

describe("chipLabel", () => {
  it("strips directories, date prefix, and extension", () => {
    expect(chipLabel("/x/2026-06-12-auth-research.md")).toBe("auth-research");
    expect(chipLabel("/x/AUTH-12-notes.md")).toBe("AUTH-12-notes");
    expect(chipLabel("/x/2026-06-12.md")).toBe("2026-06-12");
  });
});

describe("WorkstreamDocChips", () => {
  beforeEach(() => {
    vi.mocked(commands.listWorkstreamDocs).mockReturnValue(ok([RESEARCH, PLAN]) as never);
    vi.mocked(commands.detachWorkstreamDoc).mockReturnValue(ok([PLAN]) as never);
  });

  it("renders one chip per attached doc", async () => {
    renderChips();
    expect(await screen.findByText("auth-research")).toBeTruthy();
    expect(screen.getByText("auth-plan")).toBeTruthy();
    // Attach affordance shows the short label once docs exist.
    expect(screen.getByText("Attach")).toBeTruthy();
  });

  it("opens a doc when its chip is clicked", async () => {
    const { onOpenDoc } = renderChips();
    fireEvent.click(await screen.findByText("auth-research"));
    expect(onOpenDoc).toHaveBeenCalledWith(RESEARCH.docPath);
  });

  it("detaches via the chip's close button", async () => {
    renderChips();
    fireEvent.click(await screen.findByLabelText("Detach auth-research"));
    await waitFor(() =>
      expect(commands.detachWorkstreamDoc).toHaveBeenCalledWith("ws1", RESEARCH.docPath),
    );
  });

  it("offers 'Attach context' and browse-jump when nothing is attached", async () => {
    vi.mocked(commands.listWorkstreamDocs).mockReturnValue(ok([]) as never);
    const { onBrowse } = renderChips();
    fireEvent.click(await screen.findByText("Attach context"));
    expect(onBrowse).toHaveBeenCalled();
  });
});
