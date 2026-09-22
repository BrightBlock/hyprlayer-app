export const IS_MACOS =
  typeof navigator !== "undefined" && /Macintosh|Mac OS X/.test(navigator.userAgent);

export const IS_LINUX =
  typeof navigator !== "undefined" && /Linux|X11/.test(navigator.userAgent);
