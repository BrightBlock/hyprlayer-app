import type { CSSProperties } from "react";

export function HyprlayerLogo({ height = 28 }: { height?: number }) {
  return (
    <span className="relative inline-flex select-none items-center" aria-label="Hyprlayer">
      <span className="absolute right-full mr-2.5 flex items-center">
        <HyprlayerMark size={height} />
      </span>
      <span
        className="font-semibold tracking-tight text-ink"
        style={{ fontSize: height * 0.62, lineHeight: 1 }}
      >
        Hyprlayer
      </span>
    </span>
  );
}

export function HyprlayerMark({
  size = 28,
  animated = false,
  tone = "accent",
}: {
  size?: number;
  animated?: boolean;
  tone?: "accent" | "muted";
}) {
  const cls = animated ? "hl-layer" : undefined;
  const fill = tone === "muted" ? "var(--color-ink-dim)" : "var(--color-accent)";
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 32 32"
      fill="none"
    >
      <path
        d="M16 26 4 20l12-6 12 6-12 6Z"
        fill={fill}
        className={cls}
        opacity={animated ? undefined : 0.35}
        style={animated ? ({ "--hl-base": 0.35 } as CSSProperties) : undefined}
      />
      <path
        d="M16 22 4 16l12-6 12 6-12 6Z"
        fill={fill}
        className={cls}
        opacity={animated ? undefined : 0.6}
        style={animated ? ({ "--hl-base": 0.6 } as CSSProperties) : undefined}
      />
      <path
        d="M16 18 4 12l12-6 12 6-12 6Z"
        fill={fill}
        className={cls}
        style={animated ? ({ "--hl-base": 1 } as CSSProperties) : undefined}
      />
    </svg>
  );
}
