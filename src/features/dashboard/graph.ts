import type { DocMeta } from "@/lib/bindings";
import { docTitle } from "@/features/library/text";

type GraphNode = {
  id: string;
  title: string;
  type: string;
  degree: number;
};

type GraphEdge = { source: string; target: string; kind: "ref" | "topic" };

export type Graph = { nodes: GraphNode[]; edges: GraphEdge[] };

const TOPIC_GROUP_MAX = 6;

const NEBULA_COLORS = [
  "var(--color-nebula-1)",
  "var(--color-nebula-2)",
  "var(--color-nebula-3)",
  "var(--color-nebula-4)",
  "var(--color-nebula-5)",
];

const TYPE_COLORS: Record<string, string> = {
  research: "var(--color-research)",
  plan: "var(--color-plan)",
  handoff: "var(--color-handoff)",
  pr: "var(--color-ship)",
  note: "var(--color-commit)",
};

export function typeColor(type: string): string {
  const known = TYPE_COLORS[type];
  if (known) return known;
  if (type === "other") return NEBULA_COLORS[2];
  let h = 0;
  for (let i = 0; i < type.length; i++) h = (h * 31 + type.charCodeAt(i)) >>> 0;
  return NEBULA_COLORS[h % NEBULA_COLORS.length];
}

function normalize(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/^\[\[/, "")
    .replace(/\]\]$/, "");
}

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

function relatedCandidates(related: string): string[] {
  const n = normalize(related);
  const base = n.split("/").pop() ?? n;
  return unique([n, base, base.replace(/\.md$/i, "")]);
}

function unique(values: string[]): string[] {
  return Array.from(new Set(values.filter(Boolean)));
}

export function buildGraph(docs: DocMeta[]): Graph {
  const nodes: GraphNode[] = docs.map((doc) => ({
    id: doc.id,
    title: docTitle(doc),
    type: (doc.fm.type ?? "other").toLowerCase(),
    degree: 0,
  }));

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

  const addEdge = (a: string, b: string, kind: GraphEdge["kind"]): void => {
    if (a === b) return;
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

type SimulateOptions = {
  width: number;
  height: number;
  iterations?: number;
};

export function simulate(graph: Graph, opts: SimulateOptions): Point[] {
  const { width, height } = opts;
  const iterations = opts.iterations ?? 300;
  const n = graph.nodes.length;
  if (n === 0) return [];
  if (n === 1) return [{ x: width / 2, y: height / 2 }];

  const index = new Map<string, number>();
  graph.nodes.forEach((node, i) => index.set(node.id, i));

  const area = width * height;
  const k = Math.sqrt(area / n);
  const cx = width / 2;
  const cy = height / 2;
  const seed = Math.min(width, height) / 4;

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
  const gravity = 0.04;

  for (let iter = 0; iter < iterations; iter++) {
    const temp = maxStep * (1 - iter / iterations);
    for (const d of disp) {
      d.x = 0;
      d.y = 0;
    }

    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        let dx = pos[i].x - pos[j].x;
        let dy = pos[i].y - pos[j].y;
        let dist = Math.hypot(dx, dy);
        if (dist < 0.01) {
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
