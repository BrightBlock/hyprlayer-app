// @vitest-environment jsdom
import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { create } from "zustand";

import type { TranscriptItem } from "@/features/session/transcript";

const store = vi.hoisted(() => ({ current: null as unknown as ReturnType<typeof makeStore> }));
function makeStore() {
  return create<{ sessions: Record<string, { items: TranscriptItem[]; stage: string; live: boolean; workstreamId: string }> }>(
    () => ({ sessions: {} }),
  );
}
vi.mock("@/features/session/sessionStore", () => ({
  useSessionStore: <T,>(select: (s: ReturnType<ReturnType<typeof makeStore>["getState"]>) => T) => store.current(select),
}));
vi.mock("@/lib/bindings", () => ({ events: {}, commands: {} }));

const { useUiStore } = await import("@/lib/uiStore");
const { usePlanPreviewStore } = await import("./planPreviewStore");
const { usePlanAside } = await import("./usePlanAside");

const PLAN = "/t/repos/app/shared/plans/2026-09-24-og.md";
const user: TranscriptItem = { kind: "user", id: "u", text: "/create_plan", stage: null, attachments: [], createdAt: null, promptName: null };
const write: TranscriptItem = {
  kind: "tool",
  id: "w",
  toolUseId: "tu-w",
  name: "Write",
  input: JSON.stringify({ file_path: PLAN, content: "" }),
  result: { content: "ok", isError: false },
};

let sid = 0;
function session(items: TranscriptItem[], extra: Partial<{ stage: string; live: boolean }> = {}) {
  const id = `s${sid++}`;
  store.current.setState((s) => ({
    sessions: { ...s.sessions, [id]: { items, stage: "plan", live: true, workstreamId: "w1", ...extra } },
  }));
  return id;
}
function writePlan(id: string) {
  act(() =>
    store.current.setState((s) => ({ sessions: { ...s.sessions, [id]: { ...s.sessions[id], items: [user, write] } } })),
  );
}

beforeEach(() => {
  store.current = makeStore();
  useUiStore.setState({ asideView: "changes", asideViewChosen: false, diffHidden: false, narrowDrawer: null });
  usePlanPreviewStore.setState({ bySession: {}, returnTo: null, narrow: false });
});
afterEach(cleanup);

describe("usePlanAside auto-open", () => {
  it("opens when the aside is closed and a plan is first written", () => {
    useUiStore.setState({ diffHidden: true });
    const id = session([user]);
    renderHook(() => usePlanAside(id, false));
    writePlan(id);
    expect(useUiStore.getState()).toMatchObject({ asideView: "plan", diffHidden: false });
    expect(usePlanPreviewStore.getState().bySession[id]).toMatchObject({ key: PLAN, followLatest: true });
  });

  it("takes over the default Changes view", () => {
    const id = session([user]);
    renderHook(() => usePlanAside(id, false));
    writePlan(id);
    expect(useUiStore.getState().asideView).toBe("plan");
  });

  it("never steals a Changes view the user opened", () => {
    useUiStore.getState().setAsideView("changes");
    const id = session([user]);
    renderHook(() => usePlanAside(id, false));
    writePlan(id);
    expect(useUiStore.getState().asideView).toBe("changes");
  });

  it("does not open for a plan that was already written when the session loaded", () => {
    const id = session([user, write]);
    renderHook(() => usePlanAside(id, false));
    expect(useUiStore.getState().asideView).toBe("changes");
  });

  it("does not open when a restored transcript arrives with its plan already in it", () => {
    const id = session([]);
    renderHook(() => usePlanAside(id, false));
    writePlan(id);
    expect(useUiStore.getState().asideView).toBe("changes");
  });

  it("does not open for a session that is not live", () => {
    const id = session([user], { live: false });
    renderHook(() => usePlanAside(id, false));
    writePlan(id);
    expect(useUiStore.getState().asideView).toBe("changes");
  });

  it("opens once per session", () => {
    const id = session([user]);
    const { rerender } = renderHook(() => usePlanAside(id, false));
    writePlan(id);
    act(() => useUiStore.setState({ asideView: "changes" }));
    writePlan(id);
    rerender();
    expect(useUiStore.getState().asideView).toBe("changes");
  });

  it("opens the narrow drawer when narrow", () => {
    const id = session([user]);
    renderHook(() => usePlanAside(id, true));
    writePlan(id);
    expect(useUiStore.getState()).toMatchObject({ asideView: "plan", narrowDrawer: "diff" });
  });
});
