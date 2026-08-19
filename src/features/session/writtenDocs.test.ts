import { beforeEach, describe, expect, it } from "vitest";

import { forgetWrittenDocs, recordWrittenDocs, useWrittenDocsStore } from "./writtenDocs";

const byName = () => useWrittenDocsStore.getState().byName;

beforeEach(() => {
  useWrittenDocsStore.setState({ byName: {} });
});

describe("recordWrittenDocs", () => {
  it("keys doc ids by filename", () => {
    recordWrittenDocs(["/t/shared/plans/2026-08-16-a.md", "/t/shared/research/b.md"]);
    expect(byName()).toEqual({
      "2026-08-16-a.md": "/t/shared/plans/2026-08-16-a.md",
      "b.md": "/t/shared/research/b.md",
    });
  });

  it("ignores paths outside the doc taxonomy", () => {
    recordWrittenDocs(["/repo/README.md", "/t/shared/notes/x.md", "/t/shared/plans/y.txt"]);
    expect(byName()).toEqual({});
  });

  it("lets a later write of the same filename win", () => {
    recordWrittenDocs(["/t/a/plans/dup.md"]);
    recordWrittenDocs(["/t/b/plans/dup.md"]);
    expect(byName()["dup.md"]).toBe("/t/b/plans/dup.md");
  });

  // A re-write must not keep its original insertion position, or a doc still in
  // active use ages out of the bounded map while newer, idle docs survive.
  it("bounds the map, evicting least-recently-written first", () => {
    const id = (n: number) => `/t/shared/plans/doc-${n}.md`;
    recordWrittenDocs(Array.from({ length: 500 }, (_, i) => id(i)));
    recordWrittenDocs([id(0)]); // touch the oldest
    recordWrittenDocs(["/t/shared/plans/fresh.md"]);

    const names = Object.keys(byName());
    expect(names).toHaveLength(500);
    expect(names).toContain("doc-0.md"); // touched, so it survived
    expect(names).not.toContain("doc-1.md"); // next-oldest was evicted instead
    expect(names).toContain("fresh.md");
  });
});

describe("forgetWrittenDocs", () => {
  it("drops a deleted doc so its path stops resolving", () => {
    recordWrittenDocs(["/t/shared/plans/a.md"]);
    forgetWrittenDocs(["/t/shared/plans/a.md"]);
    expect(byName()).toEqual({});
  });

  it("keeps the entry when another id owns that filename now", () => {
    recordWrittenDocs(["/t/a/plans/dup.md"]);
    recordWrittenDocs(["/t/b/plans/dup.md"]);
    forgetWrittenDocs(["/t/a/plans/dup.md"]); // the stale one, already superseded
    expect(byName()["dup.md"]).toBe("/t/b/plans/dup.md");
  });
});
