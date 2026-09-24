// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/bindings", () => ({ commands: { getDoc: vi.fn(() => new Promise(() => {})) }, events: {} }));

const { useUiStore } = await import("@/lib/uiStore");
const { closePlanPreview, openPlanPreview, usePlanPreviewStore } = await import("./planPreviewStore");
const { useWrittenDocsStore } = await import("@/features/session/writtenDocs");
const { PlanCard } = await import("./PlanCard");

const PATH = "/t/repos/app/shared/plans/2026-09-24-og.md";
const CONTENT = "---\nstatus: draft\n---\n# OG images\n\n## Phase 1: A\n- [ ] a\n\n## Phase 2: B\n- [ ] b\n";

let client: QueryClient;
function wrap(node: ReactNode) {
  return <QueryClientProvider client={client}>{node}</QueryClientProvider>;
}
function renderCards() {
  return render(
    wrap(
      <>
        <PlanCard sessionId="s1" workstreamId="w1" toolUseId="t1" planKey={PATH} text={null} superseded latest={false} />
        <PlanCard sessionId="s1" workstreamId="w1" toolUseId="t2" planKey={PATH} text={null} superseded={false} latest />
      </>,
    ),
  );
}
const card = (id: string) => document.querySelector<HTMLButtonElement>(`[data-plan-card="${id}"]`)!;

beforeEach(() => {
  client = new QueryClient();
  client.setQueryData(["doc", PATH], { content: CONTENT, mtimeMs: Date.now() - 120_000, meta: {} });
  useUiStore.setState({ asideView: "changes", asideViewChosen: false, diffHidden: false, narrowDrawer: null });
  usePlanPreviewStore.setState({ bySession: {}, returnTo: null, narrow: false });
});
afterEach(cleanup);

describe("PlanCard", () => {
  it("shows the doc's title, phase count and age, with a preview hint", () => {
    renderCards();
    const latest = card("t2");
    expect(latest.getAttribute("aria-label")).toBe("Preview plan: OG images");
    expect(latest.textContent).toContain("Plan · 2 phases · 2m ago");
    expect(latest.textContent).toContain("click to preview →");
    expect(latest.getAttribute("aria-expanded")).toBe("false");
    expect(latest.className).toContain("border-edge");
  });

  it("labels older revisions superseded", () => {
    renderCards();
    expect(card("t1").textContent).toContain("Plan · superseded");
    expect(card("t1").textContent).not.toContain("phases");
  });

  it("goes active on click: ink border, ring, hint hidden, panel opened", () => {
    renderCards();
    fireEvent.click(card("t2"));
    const latest = card("t2");
    expect(latest.getAttribute("aria-expanded")).toBe("true");
    expect(latest.className).toContain("border-ink");
    expect(latest.className).toContain("shadow-[0_0_0_3px_#11111110]");
    expect(latest.textContent).not.toContain("click to preview");
    expect(useUiStore.getState().asideView).toBe("plan");
    expect(card("t1").getAttribute("aria-expanded")).toBe("false");
  });

  it("closes the panel when the active card is clicked again", () => {
    renderCards();
    fireEvent.click(card("t2"));
    fireEvent.click(card("t2"));
    expect(useUiStore.getState().asideView).toBe("changes");
    expect(card("t2").getAttribute("aria-expanded")).toBe("false");
  });

  it("a superseded card opens the latest plan and is the one marked active", () => {
    renderCards();
    fireEvent.click(card("t1"));
    expect(usePlanPreviewStore.getState().bySession.s1).toEqual({ key: PATH, cardId: "t1", followLatest: false });
    expect(card("t1").getAttribute("aria-expanded")).toBe("true");
    expect(card("t2").getAttribute("aria-expanded")).toBe("false");
  });

  it("marks the newest revision active when the preview is open without a picked card", () => {
    renderCards();
    act(() => useUiStore.setState({ asideView: "plan" }));
    expect(card("t2").getAttribute("aria-expanded")).toBe("true");
    expect(card("t1").getAttribute("aria-expanded")).toBe("false");
  });

  it("is inactive while the aside is hidden, even if selected", () => {
    renderCards();
    fireEvent.click(card("t2"));
    act(() => useUiStore.setState({ diffHidden: true }));
    expect(card("t2").getAttribute("aria-expanded")).toBe("false");
  });

  it("closing from the panel restores the prior aside and focuses the active card", () => {
    useUiStore.setState({ diffHidden: true });
    renderCards();
    fireEvent.click(card("t2"));
    expect(useUiStore.getState().diffHidden).toBe(false);
    act(() => closePlanPreview());
    expect(useUiStore.getState()).toMatchObject({ asideView: "changes", diffHidden: true });
    expect(document.activeElement).toBe(card("t2"));
  });

  it("stays active when the preview was opened on the worktree symlink path", () => {
    useWrittenDocsStore.setState({ byWorkstream: { w1: { "2026-09-24-og.md": PATH } } });
    renderCards();
    act(() =>
      openPlanPreview("s1", {
        key: "/wt/test-0cea/thoughts/shared/plans/2026-09-24-og.md",
        cardId: null,
        followLatest: true,
      }),
    );
    expect(card("t2").getAttribute("aria-expanded")).toBe("true");
  });

  it("falls back to the file name before the doc loads", () => {
    client.removeQueries({ queryKey: ["doc", PATH] });
    render(wrap(<PlanCard sessionId="s1" workstreamId="w1" toolUseId="t9" planKey={PATH} text={null} superseded={false} latest />));
    expect(screen.getByRole("button").getAttribute("aria-label")).toBe("Preview plan: og");
  });

  it("titles an inline plan from its text and leaves out the phase count and time", () => {
    render(wrap(<PlanCard sessionId="s1" workstreamId="w1" toolUseId="c1" planKey="inline" text={"# Codex plan\n\n1. Do it"} superseded={false} latest />));
    const inline = card("c1");
    expect(inline.getAttribute("aria-label")).toBe("Preview plan: Codex plan");
    expect(inline.textContent).toContain("Plan");
    expect(inline.textContent).not.toMatch(/phase|ago|just now/);
  });
});
