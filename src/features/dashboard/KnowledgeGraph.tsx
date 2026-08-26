import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useNavigate } from "@tanstack/react-router";

import { simulate, typeColor, type Graph, type Point } from "./graph";

const WIDTH = 960;
const HEIGHT = 640;
const MIN_SCALE = 0.3;
const MAX_SCALE = 6;
const STAGE_PAD = 28;
const SPREAD = 0.68;
const GLOW_NODES = 12;

type Props = {
  graph: Graph;
};

export function KnowledgeGraph({ graph: full }: Props) {
  const navigate = useNavigate();
  const svgRef = useRef<SVGSVGElement>(null);

  const graph = useMemo(() => {
    const connected = full.nodes.filter((n) => n.degree > 0);
    return connected.length > 0 ? { nodes: connected, edges: full.edges } : full;
  }, [full]);

  const [layout, setLayout] = useState<Point[] | null>(null);
  const [overrides, setOverrides] = useState<Record<string, Point>>({});
  const [hovered, setHovered] = useState<string | null>(null);
  const [view, setView] = useState({ scale: 1, tx: 0, ty: 0 });

  const viewRef = useRef(view);
  viewRef.current = view;
  const dragNode = useRef<{ id: string; moved: boolean } | null>(null);
  const pan = useRef<{ x: number; y: number; moved: boolean } | null>(null);

  useEffect(() => {
    setLayout(null);
    setOverrides({});
    setView({ scale: 1, tx: 0, ty: 0 });
    const iterations = graph.nodes.length > 140 ? 220 : 300;
    const run = () =>
      setLayout(spread(simulate(graph, { width: WIDTH, height: HEIGHT, iterations })));
    let raf = 0;
    let timer: number | undefined;
    if (document.visibilityState === "hidden") timer = window.setTimeout(run, 50);
    else raf = requestAnimationFrame(run);
    return () => {
      cancelAnimationFrame(raf);
      window.clearTimeout(timer);
    };
  }, [graph]);

  const idToIndex = useMemo(() => {
    const m = new Map<string, number>();
    graph.nodes.forEach((n, i) => m.set(n.id, i));
    return m;
  }, [graph]);

  const idToType = useMemo(() => {
    const m = new Map<string, string>();
    for (const n of graph.nodes) m.set(n.id, n.type);
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

  const { glowIds, hubId } = useMemo(() => {
    const ranked = [...graph.nodes].sort((a, b) => b.degree - a.degree);
    const glowIds = new Set(
      ranked.slice(0, GLOW_NODES).filter((n) => n.degree >= 3).map((n) => n.id),
    );
    return { glowIds, hubId: ranked[0] && ranked[0].degree > 0 ? ranked[0].id : null };
  }, [graph]);

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
  }, []);

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

  const posOf = (id: string, idx: number): Point =>
    overrides[id] ?? layout?.[idx] ?? { x: WIDTH / 2, y: HEIGHT / 2 };

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
    if (d && !d.moved) void navigate({ to: "/library", search: { doc: id } });
  };

  const resetView = () => setView({ scale: 1, tx: 0, ty: 0 });

  const active = hovered;
  const activeNeighbors = active ? neighbors.get(active) : undefined;
  const isLit = (id: string) => !active || id === active || (activeNeighbors?.has(id) ?? false);
  const { scale } = view;

  return (
    <svg
      ref={svgRef}
      viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
      preserveAspectRatio="xMidYMid meet"
      className="block h-full w-full touch-none select-none"
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onLostPointerCapture={onPointerUp}
    >
      <defs>
        <linearGradient
          id="kgf-edge"
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
        <radialGradient
          id="kgf-bg"
          cx="0.58"
          cy="0.42"
          r="1"
          gradientUnits="userSpaceOnUse"
          gradientTransform={`matrix(${WIDTH} 0 0 ${HEIGHT} 0 0)`}
        >
          <stop offset="0" stopColor="var(--color-nebula-bg-0)" />
          <stop offset="0.18" stopColor="var(--color-nebula-bg-1)" />
          <stop offset="0.42" stopColor="var(--color-surface)" />
        </radialGradient>
        <filter id="kgf-glow" x="-80%" y="-80%" width="260%" height="260%">
          <feGaussianBlur stdDeviation="6" />
        </filter>
        <radialGradient
          id="kgf-fade"
          cx="0.5"
          cy="0.44"
          r="0.56"
          gradientUnits="userSpaceOnUse"
          gradientTransform={`matrix(${WIDTH} 0 0 ${HEIGHT} 0 0)`}
        >
          <stop offset="0.28" stopColor="#fff" stopOpacity="1" />
          <stop offset="0.36" stopColor="#fff" stopOpacity="0.95" />
          <stop offset="0.44" stopColor="#fff" stopOpacity="0.83" />
          <stop offset="0.52" stopColor="#fff" stopOpacity="0.67" />
          <stop offset="0.6" stopColor="#fff" stopOpacity="0.49" />
          <stop offset="0.68" stopColor="#fff" stopOpacity="0.31" />
          <stop offset="0.76" stopColor="#fff" stopOpacity="0.16" />
          <stop offset="0.84" stopColor="#fff" stopOpacity="0.05" />
          <stop offset="0.92" stopColor="#fff" stopOpacity="0" />
        </radialGradient>
        <mask id="kgf-vignette" maskUnits="userSpaceOnUse">
          <rect
            x={-WIDTH}
            y={-HEIGHT}
            width={WIDTH * 3}
            height={HEIGHT * 3}
            fill="url(#kgf-fade)"
          />
        </mask>
        <filter id="kgf-grain">
          <feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="1" stitchTiles="stitch" />
          <feColorMatrix type="saturate" values="0" />
        </filter>
      </defs>

      <rect
        x={-WIDTH}
        y={-HEIGHT}
        width={WIDTH * 3}
        height={HEIGHT * 3}
        fill="url(#kgf-bg)"
        className="cursor-grab"
        onPointerDown={onBgPointerDown}
        onDoubleClick={resetView}
      />

      {layout && (
        <g mask="url(#kgf-vignette)">
          <g transform={`translate(${view.tx} ${view.ty}) scale(${scale})`}>
            {graph.edges.map((edge, i) => {
              const a = posOf(edge.source, idToIndex.get(edge.source) ?? 0);
              const b = posOf(edge.target, idToIndex.get(edge.target) ?? 0);
              const litEdge = !active || edge.source === active || edge.target === active;
              const base = edge.kind === "ref" ? 0.45 : 0.25;
              const typeA = idToType.get(edge.source);
              const stroke =
                typeA && typeA === idToType.get(edge.target)
                  ? typeColor(typeA)
                  : "url(#kgf-edge)";
              return (
                <path
                  key={i}
                  d={curvedPath(a, b, i)}
                  fill="none"
                  stroke={stroke}
                  strokeWidth={(edge.kind === "ref" ? 1.1 : 0.75) / scale}
                  strokeOpacity={!active ? base : litEdge ? 0.95 : 0.06}
                  style={{ transition: "stroke-opacity 160ms ease" }}
                />
              );
            })}

            {graph.nodes.map((node, idx) => {
              const p = posOf(node.id, idx);
              const r = 1.4 + Math.min(Math.sqrt(node.degree) * 0.7, 2.8);
              const color = typeColor(node.type);
              const lit = isLit(node.id);
              const showLabel = active != null && lit;
              return (
                <g
                  key={node.id}
                  transform={`translate(${p.x} ${p.y})`}
                  className="cursor-pointer"
                  opacity={lit ? 1 : 0.18}
                  style={{ transition: "opacity 160ms ease" }}
                  onPointerDown={onNodePointerDown(node.id)}
                  onPointerUp={onNodePointerUp(node.id)}
                  onPointerEnter={() => setHovered(node.id)}
                  onPointerLeave={() => setHovered((cur) => (cur === node.id ? null : cur))}
                >
                  <circle r={Math.max(r + 5, 8)} fill="transparent" />
                  {glowIds.has(node.id) && (
                    <circle
                      r={r * 2.2}
                      fill={color}
                      opacity={0.35}
                      filter="url(#kgf-glow)"
                      pointerEvents="none"
                    />
                  )}
                  {node.id === hubId && (
                    <circle
                      className="kg-pulse"
                      r={r + 4 / scale}
                      fill="none"
                      stroke={color}
                      strokeWidth={1 / scale}
                      pointerEvents="none"
                    />
                  )}
                  <circle r={r} fill={color} />
                  {showLabel && (
                    <text
                      x={r + 4 / scale}
                      y={3 / scale}
                      className="fill-ink"
                      fontSize={9 / scale}
                      style={{
                        paintOrder: "stroke",
                        stroke: "var(--color-nebula-bg-1)",
                        strokeWidth: 2.5 / scale,
                      }}
                    >
                      {truncate(node.title, 36)}
                    </text>
                  )}
                </g>
              );
            })}
          </g>
        </g>
      )}

      <rect
        x={-WIDTH * 0.25}
        y={-HEIGHT * 0.25}
        width={WIDTH * 1.5}
        height={HEIGHT * 1.5}
        filter="url(#kgf-grain)"
        opacity="0.028"
        pointerEvents="none"
      />
    </svg>
  );
}

function spread(pos: Point[]): Point[] {
  if (pos.length < 2) return pos;
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
  const sx = (WIDTH - 2 * STAGE_PAD) / (maxX - minX || 1);
  const sy = (HEIGHT - 2 * STAGE_PAD) / (maxY - minY || 1);
  const cx = WIDTH / 2;
  const cy = HEIGHT / 2;
  const rx = WIDTH / 2 - STAGE_PAD;
  const ry = HEIGHT / 2 - STAGE_PAD;
  return pos.map((p) => {
    const x = STAGE_PAD + (p.x - minX) * sx;
    const y = STAGE_PAD + (p.y - minY) * sy;
    const nx = (x - cx) / rx;
    const ny = (y - cy) / ry;
    const rho = Math.hypot(nx, ny);
    if (rho < 1e-4 || rho >= 1) return { x, y };
    const boost = Math.pow(rho, SPREAD) / rho;
    return {
      x: clamp(cx + (x - cx) * boost, STAGE_PAD, WIDTH - STAGE_PAD),
      y: clamp(cy + (y - cy) * boost, STAGE_PAD, HEIGHT - STAGE_PAD),
    };
  });
}

function curvedPath(a: Point, b: Point, index: number): string {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len = Math.hypot(dx, dy) || 1;
  const bend = Math.min(22, len * 0.16) * (index % 2 === 0 ? 1 : -1);
  const cx = (a.x + b.x) / 2 - (dy / len) * bend;
  const cy = (a.y + b.y) / 2 + (dx / len) * bend;
  return `M${a.x.toFixed(1)} ${a.y.toFixed(1)}Q${cx.toFixed(1)} ${cy.toFixed(1)} ${b.x.toFixed(1)} ${b.y.toFixed(1)}`;
}

function truncate(value: string, max: number): string {
  return value.length > max ? `${value.slice(0, max - 1)}…` : value;
}

function clamp(value: number, min: number, max: number): number {
  return value < min ? min : value > max ? max : value;
}
