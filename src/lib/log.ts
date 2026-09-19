import { invoke } from "@tauri-apps/api/core";

import { isAppError } from "./ipc";

type Level = "debug" | "info" | "warn" | "error";

// tauri-plugin-log's own command and levels; its JS package is a thin
// wrapper over exactly this call.
const LEVELS: Record<Level, number> = { debug: 2, info: 3, warn: 4, error: 5 };
const MAX_CHARS = 2000;
const REPEAT_WINDOW_MS = 60_000;

const lastWritten = new Map<string, number>();

function write(level: Level, scope: string, message: string) {
  if (!("__TAURI_INTERNALS__" in window)) return;
  const text =
    message.length > MAX_CHARS
      ? `${message.slice(0, MAX_CHARS)}… (${message.length} chars)`
      : message;
  // A render loop or a polled query can fail many times a second.
  const key = `${level}\u001f${scope}\u001f${text}`;
  const now = Date.now();
  const last = lastWritten.get(key);
  if (last !== undefined && now - last < REPEAT_WINDOW_MS) return;
  if (lastWritten.size > 500) lastWritten.clear();
  lastWritten.set(key, now);
  void invoke("plugin:log|log", { level: LEVELS[level], message: text, location: scope }).catch(
    () => {},
  );
}

/** `code: message` for an AppError, the stack for an Error. */
export function describe(e: unknown): string {
  if (isAppError(e)) return `${e.code}: ${e.message}`;
  if (e instanceof Error) {
    const head = `${e.name}: ${e.message}`;
    if (!e.stack) return head;
    return e.stack.startsWith(head) ? e.stack : `${head}\n${e.stack}`;
  }
  return String(e);
}

function line(what: string, e: unknown): string {
  return e === undefined ? what : `${what}: ${describe(e)}`;
}

/**
 * Lines in the desktop's log file, targeted `webview:<scope>`. Pass what
 * happened and the error, never arguments or payloads: those carry prompts,
 * API keys and terminal keystrokes.
 */
export const log = {
  debug: (scope: string, what: string, e?: unknown) => write("debug", scope, line(what, e)),
  info: (scope: string, what: string, e?: unknown) => write("info", scope, line(what, e)),
  warn: (scope: string, what: string, e?: unknown) => write("warn", scope, line(what, e)),
  error: (scope: string, what: string, e?: unknown) => write("error", scope, line(what, e)),
};

/** Errors nothing else catches; installed first, from `main.tsx`. */
export function installGlobalHandlers() {
  window.addEventListener("error", (event) => {
    log.error("window", "uncaught error", event.error ?? event.message);
  });
  window.addEventListener("unhandledrejection", (event) => {
    log.error("window", "unhandled rejection", event.reason);
  });
}
