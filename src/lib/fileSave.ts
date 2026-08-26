
type SaveFn = () => void;

let current: { token: object; save: SaveFn } | null = null;

export function setActiveFileSave(token: object, save: SaveFn): void {
  current = { token, save };
}

export function clearActiveFileSave(token: object): void {
  if (current?.token === token) current = null;
}

export function triggerActiveFileSave(): void {
  current?.save();
}
