/**
 * Best-effort host-OS check from the webview user agent (macOS WKWebView
 * reports "Macintosh"; Windows WebView2 reports "Windows NT"; Linux "Linux").
 *
 * Used for the few places the window chrome differs. macOS uses an overlay
 * title bar (`titleBarStyle: "Overlay"`) with the traffic lights floating over
 * the top-left, so only macOS reserves space for them; Windows/Linux put their
 * controls elsewhere and shouldn't get that inset.
 */
export const IS_MACOS =
  typeof navigator !== "undefined" && /Macintosh|Mac OS X/.test(navigator.userAgent);
