import type { CSSProperties } from "react";

const LAYERS = [
  { d: "M16 26 4 20l12-6 12 6-12 6Z", base: 0.35 },
  { d: "M16 22 4 16l12-6 12 6-12 6Z", base: 0.6 },
  { d: "M16 18 4 12l12-6 12 6-12 6Z", base: 1 },
];

export function HyprlayerMark({
  size = 28,
  animated = false,
  tone = "accent",
}: {
  size?: number;
  animated?: boolean;
  tone?: "accent" | "muted";
}) {
  const fill = tone === "muted" ? "var(--color-ink-dim)" : "var(--color-accent)";
  if (!animated) {
    return (
      <svg aria-hidden="true" width={size} height={size} viewBox="0 0 32 32" fill="none">
        {LAYERS.map(({ d, base }) => (
          <path key={d} d={d} fill={fill} opacity={base} />
        ))}
      </svg>
    );
  }
  // WebKit composites opacity on an <svg> box but repaints it every frame on an SVG <path>,
  // so each pulsing layer is its own <svg>.
  return (
    <span aria-hidden="true" className="relative inline-block shrink-0" style={{ width: size, height: size }}>
      {LAYERS.map(({ d, base }) => (
        <svg
          key={d}
          width={size}
          height={size}
          viewBox="0 0 32 32"
          fill="none"
          className="hl-layer absolute inset-0"
          style={{ "--hl-base": base } as CSSProperties}
        >
          <path d={d} fill={fill} />
        </svg>
      ))}
    </span>
  );
}
