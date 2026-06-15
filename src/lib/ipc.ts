import type { AppError } from "./bindings";

type CommandResult<T> = { status: "ok"; data: T } | { status: "error"; error: AppError };

/** Unwrap a tauri-specta command result, throwing the typed AppError. */
export async function unwrap<T>(result: Promise<CommandResult<T>>): Promise<T> {
  const r = await result;
  if (r.status === "ok") return r.data;
  throw r.error;
}

export function isAppError(e: unknown): e is AppError {
  return (
    typeof e === "object" &&
    e !== null &&
    "code" in e &&
    "message" in e &&
    typeof (e as AppError).code === "string" &&
    typeof (e as AppError).message === "string"
  );
}

/** Human-readable message for anything a command can throw. */
export function errorMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "object" && e !== null && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return String(e);
}

/** Suggested fix, when the backend supplied one. */
export function errorRemedy(e: unknown): string | null {
  if (typeof e === "object" && e !== null && "remedy" in e) {
    const remedy = (e as { remedy: unknown }).remedy;
    return remedy ? String(remedy) : null;
  }
  return null;
}
