import { IS_MACOS } from "./platform";

/**
 * In-window Liquid Glass, macOS 26 (Tahoe)'s chrome look, done in CSS.
 *
 * A native NSGlassEffectView sits behind the whole webview, so it samples what
 * is behind the *window*, not the app's own content: a maximized window ghosts
 * whatever app is behind it. `backdrop-filter` samples the DOM, which is the
 * behaviour we want, and measured free over a 9k-pixel scroll.
 *
 * Two things must both hold or nothing renders:
 *   1. the band is translucent AND blurred (this class), and
 *   2. a scroller extends up under it (`-mt-9` plus padding restoring the
 *      resting position; see the Projects band and the agent tab strip).
 * Blurring a flat colour looks opaque, which is why the h-10 window header is
 * not glass: the solid h-9 bands sit under it.
 *
 * Off macOS this is plain `bg-panel`, and the `-mt-9` lift is a visual no-op.
 * Pair with a `data-glass` attribute so index.css can key the Reduce
 * Transparency and Increase Contrast fallbacks Apple requires off it.
 */
export const GLASS_BAND = IS_MACOS
  ? "bg-panel/55 backdrop-blur-md backdrop-saturate-150 backdrop-brightness-125"
  : "bg-panel";
