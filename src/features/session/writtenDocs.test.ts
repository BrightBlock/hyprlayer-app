import { beforeEach, describe, expect, it } from "vitest";

import {
  forgetWorkstreamDocs,
  forgetWrittenDocs,
  migrateWrittenDocs,
  recordWrittenDocs,
  useWrittenDocsStore,
} from "./writtenDocs";

const slice = (workstreamId: string) => useWrittenDocsStore.getState().byWorkstream[workstreamId] ?? {};

beforeEach(() => {
  useWrittenDocsStore.setState({ byWorkstream: {} });
});

describe("recordWrittenDocs", () => {
  it("keys doc ids by filename, within the working workstream's slice", () => {
    recordWrittenDocs(["/t/shared/plans/2026-08-16-a.md", "/t/shared/research/b.md"], ["ws1"]);
    expect(slice("ws1")).toEqual({
      "2026-08-16-a.md": "/t/shared/plans/2026-08-16-a.md",
      "b.md": "/t/shared/research/b.md",
    });
  });

  it("ignores paths outside the doc taxonomy", () => {
    recordWrittenDocs(["/repo/README.md", "/t/shared/notes/x.md", "/t/shared/plans/y.txt"], ["ws1"]);
    expect(slice("ws1")).toEqual({});
  });

  it("drops ids recorded with no working workstream", () => {
    recordWrittenDocs(["/t/shared/plans/a.md"], []);
    expect(useWrittenDocsStore.getState().byWorkstream).toEqual({});
  });

  it("lets a later write of the same filename win, within a slice", () => {
    recordWrittenDocs(["/t/a/plans/dup.md"], ["ws1"]);
    recordWrittenDocs(["/t/b/plans/dup.md"], ["ws1"]);
    expect(slice("ws1")["dup.md"]).toBe("/t/b/plans/dup.md");
  });

  it("records into every concurrently-working workstream's slice", () => {
    recordWrittenDocs(["/t/shared/plans/a.md"], ["ws1", "ws2"]);
    expect(slice("ws1")["a.md"]).toBe("/t/shared/plans/a.md");
    expect(slice("ws2")["a.md"]).toBe("/t/shared/plans/a.md");
  });

  it("does not let one workstream's slice resolve another's filename", () => {
    recordWrittenDocs(["/t/a/plans/only-a.md"], ["ws1"]);
    expect(slice("ws1")["only-a.md"]).toBe("/t/a/plans/only-a.md");
    expect(slice("ws2")["only-a.md"]).toBeUndefined();
  });

  // A re-write must not keep its original insertion position, or a doc still in
  // active use ages out of the bounded slice while newer, idle docs survive.
  it("bounds each slice at 200, evicting least-recently-written first", () => {
    const id = (n: number) => `/t/shared/plans/doc-${n}.md`;
    recordWrittenDocs(
      Array.from({ length: 200 }, (_, i) => id(i)),
      ["ws1"],
    );
    recordWrittenDocs([id(0)], ["ws1"]); // touch the oldest
    recordWrittenDocs(["/t/shared/plans/fresh.md"], ["ws1"]);

    const names = Object.keys(slice("ws1"));
    expect(names).toHaveLength(200);
    expect(names).toContain("doc-0.md"); // touched, so it survived
    expect(names).not.toContain("doc-1.md"); // next-oldest was evicted instead
    expect(names).toContain("fresh.md");
  });
});

describe("forgetWrittenDocs", () => {
  it("drops a deleted doc from every slice that owns it", () => {
    recordWrittenDocs(["/t/shared/plans/a.md"], ["ws1", "ws2"]);
    forgetWrittenDocs(["/t/shared/plans/a.md"]);
    expect(slice("ws1")).toEqual({});
    expect(slice("ws2")).toEqual({});
  });

  it("keeps the entry when another id owns that filename now", () => {
    recordWrittenDocs(["/t/a/plans/dup.md"], ["ws1"]);
    recordWrittenDocs(["/t/b/plans/dup.md"], ["ws1"]);
    forgetWrittenDocs(["/t/a/plans/dup.md"]); // the stale one, already superseded
    expect(slice("ws1")["dup.md"]).toBe("/t/b/plans/dup.md");
  });
});

describe("forgetWorkstreamDocs", () => {
  it("drops only the named workstream's slice", () => {
    recordWrittenDocs(["/t/shared/plans/a.md"], ["ws1", "ws2"]);
    forgetWorkstreamDocs("ws1");
    expect(useWrittenDocsStore.getState().byWorkstream).not.toHaveProperty("ws1");
    expect(slice("ws2")["a.md"]).toBe("/t/shared/plans/a.md");
  });

  it("is a no-op for a workstream with no slice", () => {
    const before = useWrittenDocsStore.getState();
    forgetWorkstreamDocs("nope");
    expect(useWrittenDocsStore.getState()).toBe(before);
  });
});

describe("migrateWrittenDocs", () => {
  it("discards a v1 flat map on upgrade to v2", () => {
    expect(migrateWrittenDocs()).toEqual({ byWorkstream: {} });
  });
});
