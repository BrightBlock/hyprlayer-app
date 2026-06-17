import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useNavigate } from "@tanstack/react-router";

import { EmptyState, Spinner } from "../../components/ui";
import { colorForType, simulate, type Graph, type Point } from "./graph";

const WIDTH = 960;
const HEIGHT = 560;
const MIN_SCALE = 0.3;
const MAX_SCALE = 6;
/** Nodes at/above this many links are labeled even when nothing is hovered. */
const HUB_DEGREE = 6;

type Props = {
  /** The full graph (built once by the parent — we don't rebuild it here). */
  graph: Graph;
};

/**
 * Interactive, Obsidian-style node-link view of the thoughts index: every doc
 * is a node, every resolved link (an explicit `related` reference, or a shared
 * tag/ticket) an edge. Connected docs pull into clusters; node size reflects how
 * linked a doc is. Scroll to zoom, drag the background to pan, drag a node to
 * arrange, hover to spotlight a doc's neighbourhood, click to open it.
 */
export function KnowledgeGraph({ graph: full }: Props) {
  const navigate = useNavigate();
  const svgRef = useRef<SVGSVGElement>(null);

  // Show only the *linked* docs — the connection story. Isolated docs (no
  // resolved link) would otherwise bury the structure in a dot-cloud; we count
  // them instead. If nothing is linked at all, fall back to showing everything.
  const graph = useMemo(() => {
    const connected = full.nodes.filter((n) => n.degree > 0);
    return connected.length > 0 ? { nodes: connected, edges: full.edges } : full;
  }, [full]);
  const unlinkedCount = full.nodes.length - graph.nodes.length;

  // The force layout is O(n²) — heavy enough to stall first paint if run inline.
  // Defer it to after the shell paints (rAF) so the page is always instant; a
  // spinner shows for the one frame it takes to settle.
  const [layout, setLayout] = useState<Point[] | null>(null);
  const [overrides, setOverrides] = useState<Record<string, Point>>({});
  const [hovered, setHovered] = useState<string | null>(null);
  const [view, setView] = useState({ scale: 1, tx: 0, ty: 0 });

  // Refs so the imperative wheel/pointer handlers read live values without
  // re-binding listeners.
  const viewRef = useRef(view);
  viewRef.current = view;
  const dragNode = useRef<{ id: string; moved: boolean } | null>(null);
  const pan = useRef<{ x: number; y: number; moved: boolean } | null>(null);

  // New graph → recompute the layout after paint and reset drags + camera.
  useEffect(() => {
    setLayout(null);
    setOverrides({});
    setView({ scale: 1, tx: 0, ty: 0 });
    const iterations = graph.nodes.length > 140 ? 220 : 300;
    const raf = requestAnimationFrame(() =>
      setLayout(simulate(graph, { width: WIDTH, height: HEIGHT, iterations })),
    );
    return () => cancelAnimationFrame(raf);
  }, [graph]);

  const idToIndex = useMemo(() => {
    const m = new Map<string, number>();
    graph.nodes.forEach((n, i) => m.set(n.id, i));
    return m;
  }, [graph]);

  const neighbors = useMemo(() => {
    const m = new Map<string, Set<string>>();
    for (const n of graph.nodes) m.set(n.id, new Set());
    for (const e of graph.edges) {
      m.get(e.source)?.add(e.target);
      m.get(e.target)?.add(e.source);
    }
    return m;
  }, [graph]);

  const presentTypes = useMemo(() => {
    const c = new Map<string, number>();
    for (const n of graph.nodes) c.set(n.type, (c.get(n.type) ?? 0) + 1);
    return [...c.entries()].sort((a, b) => b[1] - a[1]);
  }, [graph]);

  // Client → root SVG (viewBox) coords. Independent of the pan/zoom transform,
  // which lives on the inner <g>, so it stays valid mid-gesture.
  const clientToRoot = (clientX: number, clientY: number): Point | null => {
    const svg = svgRef.current;
    const ctm = svg?.getScreenCTM();
    if (!svg || !ctm) return null;
    const pt = svg.createSVGPoint();
    pt.x = clientX;
    pt.y = clientY;
    const r = pt.matrixTransform(ctm.inverse());
    return { x: r.x, y: r.y };
  };
  const clientToGraph = (clientX: number, clientY: number): Point | null => {
    const root = clientToRoot(clientX, clientY);
    if (!root) return null;
    const v = viewRef.current;
    return { x: (root.x - v.tx) / v.scale, y: (root.y - v.ty) / v.scale };
  };

  // Wheel zoom toward the cursor. Bound manually (non-passive) so we can
  // preventDefault and not scroll the page underneath.
  useEffect(() => {
    const svg = svgRef.current;
    if (!svg) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const root = clientToRoot(e.clientX, e.clientY);
      if (!root) return;
      const v = viewRef.current;
      const gx = (root.x - v.tx) / v.scale;
      const gy = (root.y - v.ty) / v.scale;
      const scale = clamp(v.scale * Math.exp(-e.deltaY * 0.0015), MIN_SCALE, MAX_SCALE);
      setView({ scale, tx: root.x - gx * scale, ty: root.y - gy * scale });
    };
    svg.addEventListener("wheel", onWheel, { passive: false });
    return () => svg.removeEventListener("wheel", onWheel);
    // Re-bind once the <svg> actually mounts: during the deferred-layout phase
    // the component renders a spinner instead, so svgRef is null on first run.
  }, [layout]);

  // Failsafe: clear any in-flight drag/pan if the window loses focus or the OS
  // cancels the gesture (trackpad, app switch). Without this a lost pointerup
  // leaves dragNode/pan set and later moves drag stale state.
  useEffect(() => {
    const reset = () => {
      dragNode.current = null;
      pan.current = null;
    };
    window.addEventListener("blur", reset);
    window.addEventListener("pointercancel", reset);
    return () => {
      window.removeEventListener("blur", reset);
      window.removeEventListener("pointercancel", reset);
    };
  }, []);

  if (graph.nodes.length === 0) {
    return (
      <EmptyState title="No documents yet">
        Capture plans, research, and notes in your projects — they’ll appear here as a connected
        graph, linked by the <span className="font-mono">related</span> field and shared
        tags/tickets.
      </EmptyState>
    );
  }

  if (!layout) {
    return (
      <div className="flex h-full items-center justify-center">
        <Spinner />
      </div>
    );
  }

  const posOf = (id: string, idx: number): Point =>
    overrides[id] ?? layout[idx] ?? { x: WIDTH / 2, y: HEIGHT / 2 };

  const onBgPointerDown = (e: ReactPointerEvent<SVGRectElement>) => {
    const r = clientToRoot(e.clientX, e.clientY);
    if (!r) return;
    pan.current = { x: r.x, y: r.y, moved: false };
    e.currentTarget.setPointerCapture?.(e.pointerId);
  };

  const onPointerMove = (e: ReactPointerEvent<SVGSVGElement>) => {
    if (dragNode.current) {
      const g = clientToGraph(e.clientX, e.clientY);
      if (!g) return;
      dragNode.current.moved = true;
      const id = dragNode.current.id;
      setOverrides((prev) => ({ ...prev, [id]: { x: clamp(g.x, 0, WIDTH), y: clamp(g.y, 0, HEIGHT) } }));
      return;
    }
    if (pan.current) {
      const r = clientToRoot(e.clientX, e.clientY);
      if (!r) return;
      const dx = r.x - pan.current.x;
      const dy = r.y - pan.current.y;
      pan.current = { x: r.x, y: r.y, moved: true };
      setView((v) => ({ ...v, tx: v.tx + dx, ty: v.ty + dy }));
    }
  };

  const onPointerUp = () => {
    dragNode.current = null;
    pan.current = null;
  };

  const onNodePointerDown = (id: string) => (e: ReactPointerEvent<SVGGElement>) => {
    e.stopPropagation();
    (e.target as Element).setPointerCapture?.(e.pointerId);
    dragNode.current = { id, moved: false };
  };
  const onNodePointerUp = (id: string) => (e: ReactPointerEvent<SVGGElement>) => {
    e.stopPropagation();
    const d = dragNode.current;
    dragNode.current = null;
    if (d && !d.moved) void navigate({ to: "/library/doc", search: { id, mode: "preview" } });
  };

  const resetView = () => setView({ scale: 1, tx: 0, ty: 0 });

  const active = hovered;
  const activeNeighbors = active ? neighbors.get(active) : undefined;
  const isLit = (id: string) => !active || id === active || (activeNeighbors?.has(id) ?? false);
  const { scale } = view;

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      <svg
        ref={svgRef}
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        preserveAspectRatio="xMidYMid meet"
        className="h-full w-full flex-1 touch-none select-none"
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onLostPointerCapture={onPointerUp}
      >
        {/* Pan / reset surface — fills the visible viewBox, sits beneath content. */}
        <rect
          x={0}
          y={0}
          width={WIDTH}
          height={HEIGHT}
          fill="transparent"
          className="cursor-grab"
          onPointerDown={onBgPointerDown}
          onDoubleClick={resetView}
        />

        <g transform={`translate(${view.tx} ${view.ty}) scale(${scale})`}>
          {/* Edges first, so nodes sit on top. */}
          {graph.edges.map((edge, i) => {
            const a = posOf(edge.source, idToIndex.get(edge.source) ?? 0);
            const b = posOf(edge.target, idToIndex.get(edge.target) ?? 0);
            const litEdge = !active || edge.source === active || edge.target === active;
            // Resting opacity: explicit refs read a touch stronger than inferred
            // topic links. Use the lighter ink-dim (not the near-black edge color)
            // so links are legible against the dark surface without a hover.
            const base = edge.kind === "ref" ? 0.65 : 0.4;
            return (
              <line
                key={i}
                x1={a.x}
                y1={a.y}
                x2={b.x}
                y2={b.y}
                stroke={litEdge && active ? "var(--color-brand)" : "var(--color-ink-dim)"}
                strokeWidth={(edge.kind === "ref" ? 1.5 : 1) / scale}
                strokeOpacity={!active ? base : litEdge ? 0.95 : 0.07}
                style={{ transition: "stroke-opacity 160ms ease, stroke 160ms ease" }}
              />
            );
          })}

          {graph.nodes.map((node, idx) => {
            const p = posOf(node.id, idx);
            const r = 3 + Math.min(Math.sqrt(node.degree) * 1.7, 8);
            const lit = isLit(node.id);
            const showLabel =
              active === node.id ||
              (active != null && lit) ||
              (active == null && node.degree >= HUB_DEGREE);
            return (
              <g
                key={node.id}
                transform={`translate(${p.x} ${p.y})`}
                className="cursor-pointer"
                opacity={lit ? 1 : 0.2}
                style={{ transition: "opacity 160ms ease" }}
                onPointerDown={onNodePointerDown(node.id)}
                onPointerUp={onNodePointerUp(node.id)}
                onPointerEnter={() => setHovered(node.id)}
                onPointerLeave={() => setHovered((cur) => (cur === node.id ? null : cur))}
              >
                <circle
                  r={r}
                  fill={colorForType(node.type)}
                  stroke="var(--color-surface)"
                  strokeWidth={1.5 / scale}
                />
                {showLabel && (
                  <text
                    x={r + 4 / scale}
                    y={4 / scale}
                    className="fill-ink"
                    fontSize={12 / scale}
                    style={{ paintOrder: "stroke", stroke: "var(--color-surface)", strokeWidth: 3 / scale }}
                  >
                    {truncate(node.title, 36)}
                  </text>
                )}
              </g>
            );
          })}
        </g>
      </svg>

      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 border-t border-edge px-1 pt-2 text-xs text-ink-dim">
        {presentTypes.map(([type, count]) => (
          <span key={type} className="inline-flex items-center gap-1.5">
            <span
              className="inline-block h-2.5 w-2.5 rounded-full"
              style={{ backgroundColor: colorForType(type) }}
            />
            {type} <span className="text-ink-dim/60">{count}</span>
          </span>
        ))}
        <span className="ml-auto text-ink-dim/60">
          {unlinkedCount > 0 && `${unlinkedCount} unlinked hidden · `}scroll to zoom · drag to pan ·
          click to open
        </span>
      </div>
    </div>
  );
}

function truncate(value: string, max: number): string {
  return value.length > max ? `${value.slice(0, max - 1)}…` : value;
}

function clamp(value: number, min: number, max: number): number {
  return value < min ? min : value > max ? max : value;
}
