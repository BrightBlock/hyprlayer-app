// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/bindings", () => ({ commands: { getDoc: vi.fn(() => new Promise(() => {})) }, events: {} }));

const { usePlan, FLASH_MS } = await import("./usePlan");

const PATH = "/t/plans/og.md";
const V1 = "---\nstatus: draft\n---\n# OG\n\n## Overview\n\nServe.\n\n## Phase 1: A\n- [ ] a\n\n## Phase 2: B\n- [ ] b\n";

let client: QueryClient;
const wrapper = ({ children }: { children: ReactNode }) => (
  <QueryClientProvider client={client}>{children}</QueryClientProvider>
);
// react-query batches observer notifications on a zero-delay timer.
const setDoc = (content: string) => {
  act(() => client.setQueryData(["doc", PATH], { content, mtimeMs: 1, meta: {} }));
  act(() => vi.advanceTimersByTime(0));
};

beforeEach(() => {
  vi.useFakeTimers();
  client = new QueryClient();
  client.setQueryData(["doc", PATH], { content: V1, mtimeMs: 1, meta: {} });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("usePlan", () => {
  it("does not flash on the first load", () => {
    const { result } = renderHook(() => usePlan(PATH), { wrapper });
    expect(result.current.plan?.phases).toHaveLength(2);
    expect(result.current.changed.size).toBe(0);
  });

  it("re-parses when the doc changes and flags only the changed sections, then clears", () => {
    const { result } = renderHook(() => usePlan(PATH), { wrapper });
    setDoc(V1.replace("- [ ] b", "- [ ] b\n- [ ] c"));
    expect(result.current.plan?.phases[1].tasks.map((t) => t.title)).toEqual(["b", "c"]);
    expect([...result.current.changed]).toEqual(["phase:2"]);
    act(() => vi.advanceTimersByTime(FLASH_MS));
    expect(result.current.changed.size).toBe(0);
  });

  it("flags the header when the status changes", () => {
    const { result } = renderHook(() => usePlan(PATH), { wrapper });
    setDoc(V1.replace("status: draft", "status: active"));
    expect(result.current.plan?.status).toBe("active");
    expect([...result.current.changed]).toEqual(["header"]);
  });

  it("does not flash when switching to a different plan", () => {
    const other = "/t/plans/other.md";
    client.setQueryData(["doc", other], { content: V1.replace("# OG", "# Other"), mtimeMs: 1, meta: {} });
    const { result, rerender } = renderHook(({ p }) => usePlan(p), { wrapper, initialProps: { p: PATH } });
    rerender({ p: other });
    expect(result.current.plan?.title).toBe("Other");
    expect(result.current.changed.size).toBe(0);
  });
});
