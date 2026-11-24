// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { DocContent, DocMeta } from "@/lib/bindings";
import { useDocSave } from "./useDocSave";

const m = vi.hoisted(() => ({
  mutateAsync: vi.fn(),
  fetchDocFresh: vi.fn(),
}));
vi.mock("@/lib/queries", () => ({
  useSaveDocParts: () => ({ mutateAsync: m.mutateAsync, isPending: false, reset: vi.fn(), error: null }),
  fetchDocFresh: m.fetchDocFresh,
}));
vi.mock("@tanstack/react-router", () => ({ useBlocker: () => ({}) }));

const doc = (body: string, mtimeMs: number) =>
  ({
    body,
    frontmatter: "title: A\n",
    mtimeMs,
    meta: { fm: { title: "A", tags: [] } },
    fmKeys: ["title", "tags"],
  }) as unknown as DocContent;

const saved = (mtimeMs: number) => ({ mtimeMs }) as DocMeta;

describe("useDocSave", () => {
  it("keeps a property edit made while the save is in flight", async () => {
    let finish!: (meta: DocMeta) => void;
    m.mutateAsync.mockImplementation(() => new Promise<DocMeta>((r) => (finish = r)));
    m.fetchDocFresh.mockResolvedValue(doc("one", 2));
    const { result } = renderHook(() => useDocSave("d1", doc("zero", 1)));

    act(() => result.current.setBody("one"));
    let saving!: Promise<boolean>;
    act(() => {
      saving = result.current.save();
    });
    act(() => result.current.setField("tags", ["urgent"]));
    await act(async () => {
      finish(saved(2));
      await saving;
    });

    expect(result.current.draft.tags).toEqual({ kind: "list", values: ["urgent"] });
    expect(result.current.dirty).toBe(true);
  });

  it("is clean after a save whose re-read fails", async () => {
    m.mutateAsync.mockResolvedValue(saved(2));
    m.fetchDocFresh.mockRejectedValue({ code: "io", message: "read failed" });
    const { result } = renderHook(() => useDocSave("d1", doc("zero", 1)));
    act(() => result.current.setBody("one"));

    let outcome: unknown;
    await act(async () => {
      outcome = await result.current.save().catch(() => "threw");
    });

    expect(outcome).toBe(true);
    expect(result.current.dirty).toBe(false);
  });
});
