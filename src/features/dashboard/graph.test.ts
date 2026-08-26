import { describe, expect, it } from "vitest";

import type { DocMeta } from "@/lib/bindings";
import { buildGraph, simulate } from "./graph";

function doc(over: Partial<DocMeta> & { id: string }): DocMeta {
  const { fm, relPath, ...rest } = over;
  return {
    relPath: relPath ?? `${over.id}.md`,
    project: null,
    area: { kind: "shared" },
    scope: "shared",
    profile: null,
    backend: "git",
    fmWarnings: [],
    mtimeMs: null,
    sizeBytes: null,
    ...rest,
    fm: { title: null, type: null, related: [], tags: [], ...fm },
  } as DocMeta;
}

describe("buildGraph", () => {
  it("resolves a related reference by title to another doc", () => {
    const docs = [
      doc({ id: "/a.md", fm: { title: "Plan A", type: "plan", related: ["Research B"] } }),
      doc({ id: "/b.md", fm: { title: "Research B", type: "research", related: [] } }),
    ];
    const graph = buildGraph(docs);
    expect(graph.nodes).toHaveLength(2);
    expect(graph.edges).toEqual([{ source: "/a.md", target: "/b.md", kind: "ref" }]);
    expect(graph.nodes.find((n) => n.id === "/a.md")?.degree).toBe(1);
    expect(graph.nodes.find((n) => n.id === "/b.md")?.degree).toBe(1);
  });

  it("resolves by filename and wiki-link syntax, ignoring extension", () => {
    const docs = [
      doc({ id: "/x/plan.md", relPath: "x/plan.md", fm: { related: ["[[notes.md]]"] } }),
      doc({ id: "/y/notes.md", relPath: "y/notes.md" }),
    ];
    expect(buildGraph(docs).edges).toEqual([
      { source: "/x/plan.md", target: "/y/notes.md", kind: "ref" },
    ]);
  });

  it("drops unresolved references and self-links", () => {
    const docs = [
      doc({ id: "/a.md", fm: { title: "A", related: ["A", "Ghost Doc"] } }),
    ];
    const graph = buildGraph(docs);
    expect(graph.edges).toEqual([]);
    expect(graph.nodes[0].degree).toBe(0);
  });

  it("dedupes a mutual link into a single undirected edge", () => {
    const docs = [
      doc({ id: "/a.md", fm: { title: "A", related: ["B"] } }),
      doc({ id: "/b.md", fm: { title: "B", related: ["A"] } }),
    ];
    expect(buildGraph(docs).edges).toHaveLength(1);
  });

  it("does not resolve a related ref to a filename shared by multiple docs", () => {
    const docs = [
      doc({ id: "/projA/plan.md", relPath: "projA/plan.md", fm: { related: ["plan.md"] } }),
      doc({ id: "/projB/plan.md", relPath: "projB/plan.md" }),
    ];
    expect(buildGraph(docs).edges).toEqual([]);
  });

  it("still resolves a related ref by an unambiguous absolute id", () => {
    const docs = [
      doc({ id: "/projA/plan.md", relPath: "projA/plan.md", fm: { related: ["/projB/plan.md"] } }),
      doc({ id: "/projB/plan.md", relPath: "projB/plan.md" }),
    ];
    expect(buildGraph(docs).edges).toEqual([
      { source: "/projA/plan.md", target: "/projB/plan.md", kind: "ref" },
    ]);
  });

  it("links docs that share a tag as a topic edge", () => {
    const docs = [
      doc({ id: "/a.md", fm: { tags: ["auth"] } }),
      doc({ id: "/b.md", fm: { tags: ["auth"] } }),
      doc({ id: "/c.md", fm: { tags: ["other"] } }),
    ];
    const edges = buildGraph(docs).edges;
    expect(edges).toEqual([{ source: "/a.md", target: "/b.md", kind: "topic" }]);
  });

  it("forms a clique of topic edges for a shared ticket", () => {
    const docs = [
      doc({ id: "/a.md", fm: { ticket: "ENG-1" } }),
      doc({ id: "/b.md", fm: { ticket: "ENG-1" } }),
      doc({ id: "/c.md", fm: { ticket: "ENG-1" } }),
    ];
    const edges = buildGraph(docs).edges;
    expect(edges).toHaveLength(3);
    expect(edges.every((e) => e.kind === "topic")).toBe(true);
  });

  it("prefers an explicit ref over an inferred topic for the same pair", () => {
    const docs = [
      doc({ id: "/a.md", fm: { title: "A", related: ["B"], tags: ["x"] } }),
      doc({ id: "/b.md", fm: { title: "B", tags: ["x"] } }),
    ];
    expect(buildGraph(docs).edges).toEqual([
      { source: "/a.md", target: "/b.md", kind: "ref" },
    ]);
  });

  it("skips tags shared by too many docs (generic tags don't imply links)", () => {
    const docs = Array.from({ length: 14 }, (_, i) =>
      doc({ id: `/d${i}.md`, fm: { tags: ["common"] } }),
    );
    expect(buildGraph(docs).edges).toEqual([]);
  });
});

describe("simulate", () => {
  const docs = Array.from({ length: 12 }, (_, i) =>
    doc({ id: `/n${i}.md`, fm: { title: `N${i}`, related: i > 0 ? [`N${i - 1}`] : [] } }),
  );
  const graph = buildGraph(docs);

  it("returns one in-bounds point per node", () => {
    const pts = simulate(graph, { width: 400, height: 300, iterations: 120 });
    expect(pts).toHaveLength(graph.nodes.length);
    for (const p of pts) {
      expect(p.x).toBeGreaterThanOrEqual(8);
      expect(p.x).toBeLessThanOrEqual(392);
      expect(p.y).toBeGreaterThanOrEqual(8);
      expect(p.y).toBeLessThanOrEqual(292);
    }
  });

  it("is deterministic across runs", () => {
    const a = simulate(graph, { width: 400, height: 300, iterations: 120 });
    const b = simulate(graph, { width: 400, height: 300, iterations: 120 });
    expect(a).toEqual(b);
  });

  it("handles an empty graph", () => {
    expect(simulate({ nodes: [], edges: [] }, { width: 100, height: 100 })).toEqual([]);
  });
});
