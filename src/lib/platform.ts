export const IS_MACOS =
  typeof navigator !== "undefined" && /Macintosh|Mac OS X/.test(navigator.userAgent);

export const IS_LINUX =
  typeof navigator !== "undefined" && /Linux|X11/.test(navigator.userAgent);

const NATIVE_MENU_SELECTOR =
  "input, textarea, [contenteditable]:not([contenteditable='false']), [data-native-menu]";

export function wantsNativeMenu(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(NATIVE_MENU_SELECTOR) !== null;
}
