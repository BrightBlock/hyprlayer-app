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
        Hypr<span className="text-brand">layer</span>
      </span>
    </span>
  );
}

/** Just the glyph — three offset layers fanning up, the "hyper layers" idea. */
export function HyprlayerMark({ size = 28 }: { size?: number }) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 32 32"
      fill="none"
    >
      <defs>
        <linearGradient id="hl-mark" x1="4" y1="6" x2="28" y2="26" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#7dd3fc" />
          <stop offset="0.55" stopColor="#60a5fa" />
          <stop offset="1" stopColor="#a78bfa" />
        </linearGradient>
      </defs>
      {/* Bottom layer (dimmest), middle, then top — an isometric stack.
          Centered vertically in the 32×32 box (content spans y 6–26) so the
          glyph's optical center lines up with the wordmark's. */}
      <path d="M16 26 4 20l12-6 12 6-12 6Z" fill="url(#hl-mark)" opacity="0.35" />
      <path d="M16 22 4 16l12-6 12 6-12 6Z" fill="url(#hl-mark)" opacity="0.6" />
      <path d="M16 18 4 12l12-6 12 6-12 6Z" fill="url(#hl-mark)" />
    </svg>
  );
}
