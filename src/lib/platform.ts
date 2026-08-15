/**
 * Best-effort host-OS check from the webview user agent (macOS WKWebView
 * reports "Macintosh"; Windows WebView2 reports "Windows NT"; Linux WebKitGTK
 * reports "Linux").
 *
 * Almost every branch in the app is macOS vs. not, because macOS is the odd one
 * out: it keeps a native overlay title bar (`titleBarStyle: "Overlay"`) with
 * traffic lights floating over the top-left — so only it reserves that inset —
 * it's the only platform given an app menu (see `macos_menu` in
 * src-tauri/src/lib.rs), and it formats chords with glyphs. Windows and Linux
 * both run `decorations: false` with the app's own caption buttons, so prefer
 * `!IS_MACOS` over listing the other two; reach for `IS_WINDOWS` only when the
 * behaviour is genuinely Windows-specific and would be wrong on Linux.
 */
export const IS_MACOS =
  typeof navigator !== "undefined" && /Macintosh|Mac OS X/.test(navigator.userAgent);

export const IS_WINDOWS =
  typeof navigator !== "undefined" && /Windows/.test(navigator.userAgent);
