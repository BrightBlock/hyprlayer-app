import type { DocMeta } from "../../lib/bindings";
import { docTitle } from "../library/text";

/** A document, as one node in the knowledge graph. */
export type GraphNode = {
  /** Doc id (absolute path) — also the key used by edges. */
  id: string;
  title: string;
  /** Frontmatter `type`, lowercased; `"other"` when absent. */
  type: string;
  /** Number of resolved links touching this node (used to size it). */
  degree: number;
};

/** An undirected link between two docs.
 *  - `ref`   — an explicit `related` reference (a deliberate link).
 *  - `topic` — an inferred link: the two docs share a ticket or a tag. */
export type GraphEdge = { source: string; target: string; kind: "ref" | "topic" };

export type Graph = { nodes: GraphNode[]; edges: GraphEdge[] };

/** Tags shared by more than this many docs are too generic to imply a
 *  meaningful pairwise link (they'd create a dense blob), so they're skipped. */
const TOPIC_GROUP_MAX = 6;

/** Node fill per doc type. Mirrors the Library's vocabulary, with a neutral
 *  fallback for anything off-schema. Each hue binds a themed token (defined in
 *  src/index.css and deepened on the light surface) so the graph recolors per
 *  theme and its amber/green nodes stay legible on cream; the type→token mapping
 *  stays here so the SVG fills and the legend read from one source. */
export const TYPE_COLORS: Record<string, string> = {
  plan: "var(--color-graph-plan)",
  research: "var(--color-graph-research)",
  handoff: "var(--color-graph-handoff)",
  note: "var(--color-graph-note)",
  pr: "var(--color-graph-pr)",
  other: "var(--color-graph-other)",
};

export function colorForType(type: string): string {
  return TYPE_COLORS[type] ?? TYPE_COLORS.other;
}

function normalize(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/^\[\[/, "")
    .replace(/\]\]$/, "");
}

/** Every string a `related` entry might be written as, for one doc. */
function docKeys(doc: DocMeta): string[] {
  const base = doc.relPath.split("/").pop() ?? doc.relPath;
  return unique([
    doc.id.toLowerCase(),
    doc.relPath.toLowerCase(),
    base.toLowerCase(),
    base.replace(/\.md$/i, "").toLowerCase(),
    docTitle(doc).toLowerCase(),
  ]);
}

/** Candidate keys a single `related` reference could resolve against. */
function relatedCandidates(related: string): string[] {
  const n = normalize(related);
  const base = n.split("/").pop() ?? n;
  return unique([n, base, base.replace(/\.md$/i, "")]);
}

function unique(values: string[]): string[] {
  return Array.from(new Set(values.filter(Boolean)));
}

/**
 * Build the node-link graph from a doc set. Nodes are the docs themselves;
 * edges come from each doc's `related` frontmatter, resolved best-effort against
 * the other docs' ids/paths/filenames/titles. References that don't resolve to a
 * doc in this set (or point at the doc itself) are dropped, so the graph only
 * ever shows links you can actually click through.
 */
export function buildGraph(docs: DocMeta[]): Graph {
  const nodes: GraphNode[] = docs.map((doc) => ({
    id: doc.id,
    title: docTitle(doc),
    type: (doc.fm.type ?? "other").toLowerCase(),
    degree: 0,
  }));

  // key -> doc id. A key that two different docs share (e.g. a common filename
  // like "plan.md" or an identical title across projects) is *ambiguous*: we
  // drop it entirely rather than resolve a `related` reference to whichever doc
  // happened to come first in query order, which would be silent cross-project
  // misinformation. Exact ids/relPaths are effectively unique and survive.
  const keyToId = new Map<string, string>();
  const ambiguous = new Set<string>();
  for (const doc of docs) {
    for (const key of docKeys(doc)) {
      const existing = keyToId.get(key);
      if (existing === undefined) keyToId.set(key, doc.id);
      else if (existing !== doc.id) ambiguous.add(key);
    }
  }
  for (const key of ambiguous) keyToId.delete(key);

  const degree = new Map<string, number>();
  const seen = new Set<string>();
  const edges: GraphEdge[] = [];

  // Undirected, deduped. `ref` is added before `topic` and the first edge for a
  // pair wins, so an explicit reference always outranks an inferred one.
  const addEdge = (a: string, b: string, kind: GraphEdge["kind"]): void => {
    if (a === b) return;
    // Collision-proof key: doc ids are filesystem paths and may contain spaces,
    // so a plain `${a} ${b}` join is ambiguous. JSON-encoding the sorted pair
    // is unambiguous regardless of the characters in either path.
    const pairKey = JSON.stringify(a < b ? [a, b] : [b, a]);
    if (seen.has(pairKey)) return;
    seen.add(pairKey);
    edges.push({ source: a, target: b, kind });
    degree.set(a, (degree.get(a) ?? 0) + 1);
    degree.set(b, (degree.get(b) ?? 0) + 1);
  };

  for (const doc of docs) {
    for (const related of doc.fm.related ?? []) {
      let targetId: string | undefined;
      for (const candidate of relatedCandidates(related)) {
        targetId = keyToId.get(candidate);
        if (targetId) break;
      }
      if (targetId) addEdge(doc.id, targetId, "ref");
    }
  }

  // 2. Topical links: docs sharing a ticket or a (non-generic) tag cluster
  //    together. Each such group becomes a clique of `topic` edges, which
  //    dedupe against the `ref` edges above via the shared `addEdge`.
  const buckets = new Map<string, string[]>();
  const pushBucket = (key: string, id: string): void => {
    const list = buckets.get(key);
    if (list) list.push(id);
    else buckets.set(key, [id]);
  };
  for (const doc of docs) {
    const ticket = doc.fm.ticket?.trim();
    if (ticket) pushBucket(`ticket:${ticket.toLowerCase()}`, doc.id);
    for (const tag of doc.fm.tags ?? []) {
      const t = tag.trim().toLowerCase();
      if (t) pushBucket(`tag:${t}`, doc.id);
    }
  }
  for (const ids of buckets.values()) {
    if (ids.length < 2 || ids.length > TOPIC_GROUP_MAX) continue;
    for (let i = 0; i < ids.length; i++) {
      for (let j = i + 1; j < ids.length; j++) addEdge(ids[i], ids[j], "topic");
    }
  }

  for (const node of nodes) node.degree = degree.get(node.id) ?? 0;
  return { nodes, edges };
}

export type Point = { x: number; y: number };

export type SimulateOptions = {
  width: number;
  height: number;
  iterations?: number;
};

/**
 * Deterministic Fruchterman–Reingold force layout. No dependencies, no RNG —
 * nodes seed onto a circle by index so the same graph always lays out the same
 * way (stable across re-renders and unit-testable). Repulsion pushes every pair
 * apart; edges pull their endpoints together; a per-iteration cooling cap keeps
 * it from oscillating. Returns one point per node, aligned to `nodes` order and
 * clamped inside the box.
 */
export function simulate(graph: Graph, opts: SimulateOptions): Point[] {
  const { width, height } = opts;
  const iterations = opts.iterations ?? 300;
  const n = graph.nodes.length;
  if (n === 0) return [];
  if (n === 1) return [{ x: width / 2, y: height / 2 }];

  const index = new Map<string, number>();
  graph.nodes.forEach((node, i) => index.set(node.id, i));

  const area = width * height;
  const k = Math.sqrt(area / n); // ideal edge length
  const cx = width / 2;
  const cy = height / 2;
  const seed = Math.min(width, height) / 4;

  // Seed on a circle, with a deterministic golden-ratio radius jitter so the
  // initial layout isn't perfectly symmetric (symmetry can stall the solver).
  const pos: Point[] = graph.nodes.map((_, i) => {
    const angle = (2 * Math.PI * i) / n;
    const r = seed * (0.55 + 0.45 * ((i * 0.6180339887) % 1));
    return { x: cx + r * Math.cos(angle), y: cy + r * Math.sin(angle) };
  });

  const edges = graph.edges
    .map((e) => [index.get(e.source), index.get(e.target)] as const)
    .filter((pair): pair is readonly [number, number] => pair[0] != null && pair[1] != null);

  const disp: Point[] = pos.map(() => ({ x: 0, y: 0 }));
  const maxStep = Math.min(width, height) / 8;
  const gravity = 0.04; // gentle pull to center so components don't drift apart

  for (let iter = 0; iter < iterations; iter++) {
    const temp = maxStep * (1 - iter / iterations);
    for (const d of disp) {
      d.x = 0;
      d.y = 0;
    }

    // Repulsion (every pair).
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        let dx = pos[i].x - pos[j].x;
        let dy = pos[i].y - pos[j].y;
        let dist = Math.hypot(dx, dy);
        if (dist < 0.01) {
          // Coincident: nudge deterministically so the direction is defined.
          dx = (i - j) * 0.01;
          dy = 0.01;
          dist = Math.hypot(dx, dy);
        }
        const force = (k * k) / dist;
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        disp[i].x += fx;
        disp[i].y += fy;
        disp[j].x -= fx;
        disp[j].y -= fy;
      }
    }

    // Attraction (along edges).
    for (const [a, b] of edges) {
      const dx = pos[a].x - pos[b].x;
      const dy = pos[a].y - pos[b].y;
      const dist = Math.hypot(dx, dy) || 0.01;
      const force = (dist * dist) / k;
      const fx = (dx / dist) * force;
      const fy = (dy / dist) * force;
      disp[a].x -= fx;
      disp[a].y -= fy;
      disp[b].x += fx;
      disp[b].y += fy;
    }

    // Centering gravity + apply, capped by temperature. No wall clamp: letting
    // the layout settle freely (then fitting it afterward) avoids the classic
    // pile-up of nodes pinned against the borders.
    for (let i = 0; i < n; i++) {
      disp[i].x += (cx - pos[i].x) * gravity * k;
      disp[i].y += (cy - pos[i].y) * gravity * k;
      const d = Math.hypot(disp[i].x, disp[i].y) || 0.01;
      pos[i].x += (disp[i].x / d) * Math.min(d, temp);
      pos[i].y += (disp[i].y / d) * Math.min(d, temp);
    }
  }

  return fitToBox(pos, width, height, 32);
}

/** Scale + translate a settled layout uniformly so its bounding box fills the
 *  viewport (minus padding) and is centered — guarantees the graph uses the
 *  whole canvas regardless of the simulation's absolute scale. */
function fitToBox(pos: Point[], width: number, height: number, pad: number): Point[] {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const p of pos) {
    if (p.x < minX) minX = p.x;
    if (p.y < minY) minY = p.y;
    if (p.x > maxX) maxX = p.x;
    if (p.y > maxY) maxY = p.y;
  }
  const spanX = maxX - minX || 1;
  const spanY = maxY - minY || 1;
  const scale = Math.min((width - 2 * pad) / spanX, (height - 2 * pad) / spanY);
  const offsetX = pad + ((width - 2 * pad) - spanX * scale) / 2;
  const offsetY = pad + ((height - 2 * pad) - spanY * scale) / 2;
  return pos.map((p) => ({
    x: offsetX + (p.x - minX) * scale,
    y: offsetY + (p.y - minY) * scale,
  }));
}
