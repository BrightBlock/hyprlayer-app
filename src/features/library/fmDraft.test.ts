import { describe, expect, it } from "vitest";

import type { Frontmatter } from "@/lib/bindings";
import { fieldValue, fileValue, placeholder, presentKeys, setField, toPatch, type FmDraft } from "./fmDraft";

const fm: Frontmatter = {
  title: "Hello",
  status: "active",
  tags: ["a", "b"],
};
const fmKeys = ["title", "status", "tags"];

describe("setField", () => {
  // Failure mode: setting a field back to its file value leaves the draft entry in
  // place, so the doc stays dirty.
  it("drops the entry when a field is set back to its file value", () => {
    let draft: FmDraft = setField({}, fm, fmKeys, "status", "draft");
    expect(draft.status).toEqual({ kind: "str", value: "draft" });
    draft = setField(draft, fm, fmKeys, "status", "active");
    expect(draft.status).toBeUndefined();
  });

  // Failure mode: clearing a field that is absent from the file emits a `remove`.
  it("drops rather than removes when clearing a field absent from the file", () => {
    const draft = setField({}, fm, fmKeys, "author", "");
    expect(draft).toEqual({});
  });

  // Failure mode: clearing a present field does not emit a `remove`.
  it("emits a remove when clearing a field present in the file", () => {
    const draft = setField({}, fm, fmKeys, "status", "");
    expect(draft.status).toEqual({ kind: "remove" });
  });

  // Failure mode: tag equality is order-sensitive, and re-adding the same tags
  // leaves the doc dirty.
  it("compares tag lists order-sensitively", () => {
    const reordered = setField({}, fm, fmKeys, "tags", ["b", "a"]);
    expect(reordered.tags).toEqual({ kind: "list", values: ["b", "a"] });

    const sameOrder = setField({}, fm, fmKeys, "tags", ["a", "b"]);
    expect(sameOrder.tags).toBeUndefined();
  });
});

describe("toPatch", () => {
  // Failure mode: toPatch puts a key in both set and remove.
  it("never puts the same key in both set and remove", () => {
    const draft: FmDraft = {
      status: { kind: "remove" },
      title: { kind: "str", value: "New" },
      tags: { kind: "list", values: ["x"] },
    };
    const patch = toPatch(draft);
    const setKeys = new Set((patch.set ?? []).map((e) => e.key));
    for (const key of patch.remove ?? []) expect(setKeys.has(key)).toBe(false);
    expect(patch.set).toEqual([
      { key: "title", value: { kind: "str", value: "New" } },
      { key: "tags", value: { kind: "list", values: ["x"] } },
    ]);
    expect(patch.remove).toEqual(["status"]);
  });
});

describe("presentKeys", () => {
  // Failure mode: presentKeys counts a removed key, or misses an added one.
  it("subtracts a removed key", () => {
    const draft: FmDraft = { status: { kind: "remove" } };
    expect(presentKeys(fmKeys, draft)).toBe(fmKeys.length - 1);
  });

  it("adds a key not in the file", () => {
    const draft: FmDraft = { author: { kind: "str", value: "jt" } };
    expect(presentKeys(fmKeys, draft)).toBe(fmKeys.length + 1);
  });

  it("nets out an add and a remove together", () => {
    const draft: FmDraft = { status: { kind: "remove" }, author: { kind: "str", value: "jt" } };
    expect(presentKeys(fmKeys, draft)).toBe(fmKeys.length);
  });
});

describe("path-derived placeholders", () => {
  // The indexer fills a missing title/type/date from the path (frontmatter.rs:284-322).
  const derivedFm: Frontmatter = { title: "2026-01-01-anchor" };
  const noKeys: string[] = [];

  // Failure mode: the placeholder shows up as a real value.
  it("shows the derived value only as a placeholder, not as the field value", () => {
    expect(fileValue(derivedFm, noKeys, "title")).toBe("");
    expect(fieldValue(derivedFm, noKeys, {}, "title")).toBe("");
    expect(placeholder(derivedFm, noKeys, "title")).toBe("2026-01-01-anchor");
  });

  // Failure mode: typing the displayed placeholder into an absent key writes nothing.
  it("typing the displayed placeholder into the absent key writes a real value", () => {
    const draft = setField({}, derivedFm, noKeys, "title", "2026-01-01-anchor");
    expect(draft.title).toEqual({ kind: "str", value: "2026-01-01-anchor" });
  });
});
