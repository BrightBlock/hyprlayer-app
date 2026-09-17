import type { Host, HostsFile } from "@/lib/bindings";

export type { Host, HostsFile };

// The local server's profile name. A host may not take it: Phase 4 namespaces
// localStorage by the active destination and leaves `local` un-prefixed, so a
// host called `local` would silently share this machine's tabs and pins.
export const LOCAL_PROFILE = "local";

export function validateDestination(destination: string): string | null {
  const trimmed = destination.trim();
  if (!trimmed) return null;
  if (/\s/.test(trimmed)) return "cannot contain spaces";
  if (trimmed === LOCAL_PROFILE) return `cannot be "${LOCAL_PROFILE}" — that name is this machine`;
  return null;
}

export function validateHostList(
  rows: { label: string; destination: string }[],
): Map<number, { label?: string; destination?: string }> {
  const errors = new Map<number, { label?: string; destination?: string }>();
  const put = (i: number, patch: { label?: string; destination?: string }) =>
    errors.set(i, { ...errors.get(i), ...patch });
  const seen = new Map<string, number>();
  rows.forEach((row, i) => {
    const label = row.label.trim();
    const destination = row.destination.trim();
    if (!label) put(i, { label: "Label required." });
    if (!destination) {
      put(i, { destination: "SSH destination required." });
      return;
    }
    const why = validateDestination(destination);
    if (why) {
      put(i, { destination: `Destination ${why}.` });
      return;
    }
    if (seen.has(destination)) put(i, { destination: "Already used by another host." });
    else seen.set(destination, i);
  });
  return errors;
}
