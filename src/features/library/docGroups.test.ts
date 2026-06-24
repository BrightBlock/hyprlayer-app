import { describe, expect, it } from "vitest";

import type { DocMeta } from "../../lib/bindings";
import { groupDocsByType } from "./docGroups";

function doc(id: string, type: string | null, mtimeMs: number | null): DocMeta {
  return { id, fm: { type }, mtimeMs } as unknown as DocMeta;
}

describe("groupDocsByType", () => {
  it("buckets by type, preserving first-seen group order and in-group order", () => {
    const groups = groupDocsByType([
      doc("a", "research", 30),
      doc("b", "plan", 20),
      doc("c", "research", 10),
    ]);
    expect(groups.map((g) => g.type)).toEqual(["research", "plan"]);
    expect(groups[0].label).toBe("RESEARCH");
    expect(groups[0].docs.map((d) => d.id)).toEqual(["a", "c"]);
    expect(groups[1].docs.map((d) => d.id)).toEqual(["b"]);
  });

  it("reports the newest mtime per group", () => {
    const [group] = groupDocsByType([doc("a", "spec", 10), doc("b", "spec", 40), doc("c", "spec", 25)]);
    expect(group.newestMs).toBe(40);
  });

  it("normalizes type casing/whitespace and labels untyped docs", () => {
    const groups = groupDocsByType([doc("a", "  Research ", 5), doc("b", null, 5), doc("c", "", 5)]);
    expect(groups.map((g) => g.type)).toEqual(["research", ""]);
    const untyped = groups.find((g) => g.type === "");
    expect(untyped?.label).toBe("UNTYPED");
    expect(untyped?.docs.map((d) => d.id)).toEqual(["b", "c"]);
  });

  it("tolerates a group with no mtimes", () => {
    const [group] = groupDocsByType([doc("a", "note", null)]);
    expect(group.newestMs).toBeNull();
  });
});
