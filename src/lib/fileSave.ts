// Active file editor's save action, so the "File ▸ Save" menu (a menu:save-file
// event wired in Root.tsx) can reach it without prop-threading.

type SaveFn = () => void;

let current: { token: object; save: SaveFn } | null = null;

/** Register `save`, owned by `token` (the editor instance); last write wins. */
export function setActiveFileSave(token: object, save: SaveFn): void {
  current = { token, save };
}

/** Release the registration if `token` still owns it. */
export function clearActiveFileSave(token: object): void {
  if (current?.token === token) current = null;
}

export function triggerActiveFileSave(): void {
  current?.save();
}
