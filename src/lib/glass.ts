import { IS_MACOS } from "./platform";

/**
 * In-window Liquid Glass — macOS 26 (Tahoe)'s chrome look, done in CSS.
 *
 * Apple's glass samples the app's *own* content scrolling beneath it. A native
 * NSGlassEffectView can't do that from a Tauri app: it sits behind the whole
 * webview, so it only ever sees what's behind the *window* — a maximized window
 * ends up ghosting whatever app is behind it. `backdrop-filter` samples the DOM
 * instead, which is precisely the in-window behaviour, and measured at no cost
 * over a 9k-pixel transcript scroll.
 *
 * Two things must both hold or this renders nothing at all:
 *   1. the band is translucent AND blurred (this class), and
 *   2. a scroller extends up underneath it (`-mt-9` + padding restoring the
 *      resting position — see the Projects band and the agent tab strip).
 * A band with only flat colour beneath it blurs that flat colour and looks
 * identical to an opaque one. That is why the h-11 window header is NOT glass:
 * the h-9 bands sit under it, and they're solid.
 *
 * Off macOS this is just `bg-panel`, so Windows/Linux keep the bands they had.
 * The `-mt-9` lift stays unconditional at the call sites: with a solid band it's
 * visually a no-op (content scrolls under an opaque bar, exactly as before).
 *
 * Pair with a `data-glass` attribute on the same element — index.css keys the
 * Reduce Transparency / Increase Contrast fallback off it, which Apple requires
 * glass surfaces to honor and which CSS blur doesn't get for free.
 */
export const GLASS_BAND = IS_MACOS
  ? "bg-panel/55 backdrop-blur-md backdrop-saturate-150 backdrop-brightness-125"
  : "bg-panel";
