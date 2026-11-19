// @vitest-environment jsdom
import { cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClientProvider } from "@tanstack/react-query";
import { createElement, type ReactNode } from "react";

import type { ThoughtQuery } from "@/lib/bindings";

const cmds = vi.hoisted(() => ({
  queryDocs: vi.fn((_q: unknown) =>
    Promise.resolve({ status: "ok", data: { items: [{ id: "readme" }], total: 1 } }),
  ),
}));
vi.mock("@/lib/bindings", () => ({ commands: cmds }));

import { queryClient } from "@/lib/queryClient";
import { useDocs } from "@/lib/queries";

const wrapper = ({ children }: { children: ReactNode }) =>
  createElement(QueryClientProvider, { client: queryClient }, children);

afterEach(() => {
  cleanup();
  queryClient.clear();
});

describe("useDocs", () => {
  it("drops the last results once a new query is disabled", async () => {
    const search = { text: "READ", limit: 12 } as unknown as ThoughtQuery;
    const idle = { text: null, limit: 12 } as unknown as ThoughtQuery;
    const { result, rerender } = renderHook(({ q, on }) => useDocs(q, on), {
      wrapper,
      initialProps: { q: search, on: true },
    });
    await waitFor(() => expect(result.current.data?.items).toHaveLength(1));

    rerender({ q: idle, on: false });
    expect(result.current.data).toBeUndefined();
  });

  it("keeps the last results while an enabled query refetches", async () => {
    const first = { text: "READ", limit: 12 } as unknown as ThoughtQuery;
    const next = { text: "READM", limit: 12 } as unknown as ThoughtQuery;
    cmds.queryDocs.mockImplementationOnce(() =>
      Promise.resolve({ status: "ok", data: { items: [{ id: "readme" }], total: 1 } }),
    );
    const { result, rerender } = renderHook(({ q }) => useDocs(q, true), {
      wrapper,
      initialProps: { q: first },
    });
    await waitFor(() => expect(result.current.data?.items).toHaveLength(1));

    cmds.queryDocs.mockImplementationOnce(() => new Promise(() => {}));
    rerender({ q: next });
    expect(result.current.data?.items).toHaveLength(1);
  });
});
