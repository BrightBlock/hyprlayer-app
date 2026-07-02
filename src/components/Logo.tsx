import type { CSSProperties } from "react";

/**
 * The Hyprlayer brand lockup: a stacked-"layers" glyph next to the wordmark.
 * Pure inline SVG (currentColor-aware where it helps) so it stays crisp at any
 * DPI and themes with the dark UI without shipping a raster asset.
 */
export function HyprlayerLogo({ height = 28 }: { height?: number }) {
  return (
    // The wordmark is the centered anchor; the mark hangs out-of-flow to its
    // left (position: absolute), so when this lockup sits in a centered column
    // the *text* is what lands on the page's center line — not the icon+text
    // midpoint, which would push the word off to the right.
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

/**
 * Just the glyph — three offset layers fanning up, the "hyper layers" idea.
 * Terracotta like hyprlayer.dev's mark: the site fills #d97757 flat; here the
 * accent token supplies that value on the dark themes and each theme's
 * design-system accent elsewhere (deepened on light, green on Matrix, …).
 *
 * Pass `animated` while a session is working: each layer dims to half its
 * resting weight and lights back up in a bottom→top wave, reading as live work
 * without the urgency of a spinner. Each layer's resting opacity is fed to the
 * shared `hl-layer-pulse` keyframes via the `--hl-base` custom property, and the
 * stagger lives in `.hl-layer:nth-child()` delays — both in index.css, which
 * also pins the layers static under `prefers-reduced-motion`.
 */
export function HyprlayerMark({ size = 28, animated = false }: { size?: number; animated?: boolean }) {
  const cls = animated ? "hl-layer" : undefined;
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 32 32"
      fill="none"
    >
      {/* Bottom layer (dimmest), middle, then top — an isometric stack.
          Centered vertically in the 32×32 box (content spans y 6–26) so the
          glyph's optical center lines up with the wordmark's. When animated, the
          resting opacity moves to `--hl-base` so the keyframes can drive it. */}
      <path
        d="M16 26 4 20l12-6 12 6-12 6Z"
        fill="var(--color-accent)"
        className={cls}
        opacity={animated ? undefined : 0.35}
        style={animated ? ({ "--hl-base": 0.35 } as CSSProperties) : undefined}
      />
      <path
        d="M16 22 4 16l12-6 12 6-12 6Z"
        fill="var(--color-accent)"
        className={cls}
        opacity={animated ? undefined : 0.6}
        style={animated ? ({ "--hl-base": 0.6 } as CSSProperties) : undefined}
      />
      <path
        d="M16 18 4 12l12-6 12 6-12 6Z"
        fill="var(--color-accent)"
        className={cls}
        style={animated ? ({ "--hl-base": 1 } as CSSProperties) : undefined}
      />
    </svg>
  );
}
