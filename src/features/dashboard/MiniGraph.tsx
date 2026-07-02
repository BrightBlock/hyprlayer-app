import { useMemo } from "react";

import { nebulaColorAt, simulate, type Graph, type GraphNode, type Point } from "./graph";

const WIDTH = 368;
const HEIGHT = 170;
/** Enough dots to read as a constellation without becoming a blob. */
const MAX_NODES = 14;
/** The most-linked nodes get glow halos; the rest stay small dots. */
const MAJOR_NODES = 5;

/** Map each axis independently onto the padded box, so the layout fills the
 *  wide strip instead of sitting as a centered blob. */
function stretchToBox(points: Point[], width: number, height: number, pad: number): Point[] {
  if (points.length === 0) return points;
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const p of points) {
    minX = Math.min(minX, p.x);
    minY = Math.min(minY, p.y);
    maxX = Math.max(maxX, p.x);
    maxY = Math.max(maxY, p.y);
  }
  const spanX = maxX - minX || 1;
  const spanY = maxY - minY || 1;
  return points.map((p) => ({
    x: pad + ((p.x - minX) / spanX) * (width - 2 * pad),
    y: pad + ((p.y - minY) / spanY) * (height - 2 * pad),
  }));
}

/** A gently curved edge: quadratic bezier bowed perpendicular to the chord,
 *  alternating side by index so the mesh weaves instead of fanning one way. */
function curvedPath(a: Point, b: Point, index: number): string {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len = Math.hypot(dx, dy) || 1;
  const bend = Math.min(16, len * 0.18) * (index % 2 === 0 ? 1 : -1);
  const cx = (a.x + b.x) / 2 - (dy / len) * bend;
  const cy = (a.y + b.y) / 2 + (dx / len) * bend;
  return `M${a.x.toFixed(1)} ${a.y.toFixed(1)}Q${cx.toFixed(1)} ${cy.toFixed(1)} ${b.x.toFixed(1)} ${b.y.toFixed(1)}`;
}

type PlacedNode = GraphNode & Point & { r: number; color: string; major: boolean };

/**
 * "Nebula" preview of the knowledge graph (design pass 5a): the most-linked
 * docs as stars in the theme's nebula family — glow halos on the hubs, curved gradient
 * edges, two layers drifting on offset periods, and a pulse + label on the
 * most-linked doc. Deterministic layout (same solver as the full view);
 * decorative — the parent supplies the click affordance.
 */
export function MiniGraph({ graph }: { graph: Graph }) {
  const { nodes, edges, paths } = useMemo(() => {
    // Most-linked first (they draw the constellation); recency order breaks
    // ties, so a linkless corpus still previews as its newest docs.
    const ranked = [...graph.nodes].sort((a, b) => b.degree - a.degree).slice(0, MAX_NODES);
    const keep = new Set(ranked.map((n) => n.id));
    const edges = graph.edges.filter((e) => keep.has(e.source) && keep.has(e.target));
    const points = stretchToBox(
      simulate({ nodes: ranked, edges }, { width: WIDTH, height: HEIGHT, iterations: 150 }),
      WIDTH,
      HEIGHT,
      24,
    );
    const nodes: PlacedNode[] = ranked.map((node, i) => ({
      ...node,
      ...points[i],
      major: i < MAJOR_NODES && node.degree > 0,
      r:
        i < MAJOR_NODES && node.degree > 0
          ? 3 + Math.min(node.degree, 12) * 0.14
          : 2.2 + (i % 3) * 0.3,
      color: nebulaColorAt(points[i]?.x ?? 0, WIDTH),
    }));
    const index = new Map(ranked.map((n, i) => [n.id, i]));
    const paths = edges.map((e, i) =>
      curvedPath(points[index.get(e.source)!], points[index.get(e.target)!], i),
    );
    return { nodes, edges, paths };
  }, [graph]);

  const hub = nodes.length > 0 && nodes[0].major ? nodes[0] : null;

  const renderNode = (node: PlacedNode) => (
    <g key={node.id}>
      {node.major && (
        <circle
          cx={node.x}
          cy={node.y}
          r={node.r * 2.4}
          fill={node.color}
          opacity={0.35}
          filter="url(#kg-glow)"
        />
      )}
      <circle
        cx={node.x}
        cy={node.y}
        r={node.r}
        fill={node.color}
        opacity={node.major ? 1 : 0.8}
      />
    </g>
  );

  return (
    <div className="kg-dial relative w-full">
      <svg viewBox={`0 0 ${WIDTH} ${HEIGHT}`} className="block w-full" aria-hidden="true">
        <defs>
          <linearGradient
            id="kg-edge"
            x1="0"
            y1="0"
            x2={WIDTH}
            y2={HEIGHT}
            gradientUnits="userSpaceOnUse"
          >
            <stop offset="0" stopColor="var(--color-nebula-1)" />
            <stop offset="0.55" stopColor="var(--color-nebula-3)" />
            <stop offset="1" stopColor="var(--color-nebula-5)" />
          </linearGradient>
          <radialGradient id="kg-bg" cx="0.38" cy="0.32" r="1">
            <stop offset="0" stopColor="var(--color-nebula-bg-0)" />
            <stop offset="1" stopColor="var(--color-nebula-bg-1)" />
          </radialGradient>
          <filter id="kg-glow" x="-80%" y="-80%" width="260%" height="260%">
            <feGaussianBlur stdDeviation="5" />
          </filter>
        </defs>
        <rect width={WIDTH} height={HEIGHT} rx="10" fill="url(#kg-bg)" />
        <g stroke="url(#kg-edge)" fill="none" strokeWidth="1" opacity="0.38">
          {paths.map((d, i) => (
            <path key={`${edges[i].source}→${edges[i].target}`} d={d} />
          ))}
        </g>
        <g className="kg-drift-a">{nodes.filter((_, i) => i % 2 === 0).map(renderNode)}</g>
        <g className="kg-drift-b">{nodes.filter((_, i) => i % 2 === 1).map(renderNode)}</g>
        {hub && (
          <circle
            className="kg-pulse"
            cx={hub.x}
            cy={hub.y}
            r={hub.r + 3.5}
            fill="none"
            stroke={hub.color}
            strokeWidth="1"
          />
        )}
      </svg>
      {hub && (
        <span
          className="pointer-events-none absolute inline-flex max-w-[70%] items-center gap-1.5 overflow-hidden rounded-[7px] border border-edge bg-panel/90 px-2 py-0.5 backdrop-blur-sm"
          style={{
            // Anchor on the hub's side of center so the chip grows toward the
            // middle, capped at 26% (100 − the 70% max width − 4% margin) so
            // even a max-width chip never crosses the far edge.
            ...(hub.x <= WIDTH / 2
              ? { left: `${Math.min(26, Math.max(4, (hub.x / WIDTH) * 100))}%` }
              : { right: `${Math.min(26, Math.max(4, 100 - (hub.x / WIDTH) * 100))}%` }),
            top: `${Math.min(76, Math.max(4, ((hub.y - 26) / HEIGHT) * 100))}%`,
          }}
        >
          <span className="min-w-0 truncate text-[10.5px] font-semibold text-ink">{hub.title}</span>
          <span className="shrink-0 text-[10px] text-ink-dim">
            {hub.degree} {hub.degree === 1 ? "link" : "links"}
          </span>
        </span>
      )}
    </div>
  );
}
