import type { AiStatus, AiVersionEntry, AiVersions } from "@/lib/bindings";
import type { DropdownOption } from "@/components/ui";

export type BundlePinState =
  | { kind: "unavailable" }
  | { kind: "unpinned"; installed: string | null; binary: string | null }
  | {
      kind: "pinned";
      installed: string | null;
      binary: string | null;
      pinned: string;
      drifted: boolean;
    };

export function bundlePinState(status: AiStatus | undefined): BundlePinState {
  if (!status?.installed) return { kind: "unavailable" };
  const installed = status.assetsVersion ?? null;
  const binary = status.binaryVersion ?? null;
  if (installed === null && binary === null) return { kind: "unavailable" };
  const pinned = status.pinnedVersion ?? null;
  if (pinned === null) return { kind: "unpinned", installed, binary };
  return { kind: "pinned", installed, binary, pinned, drifted: pinned !== binary };
}

export function formatPublished(iso: string | null | undefined): string | null {
  if (!iso) return null;
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) return null;
  return new Date(ms).toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

export function versionLabel(entry: AiVersionEntry): string {
  const notes: string[] = [];
  const published = formatPublished(entry.publishedAt);
  if (published) notes.push(published);
  if (entry.current) notes.push("installed");
  if (entry.pinned) notes.push("pinned");
  return notes.length > 0 ? `${entry.version} · ${notes.join(" · ")}` : entry.version;
}

export const VERSION_PICKER_LIMIT = 3;

export function versionOptions(data: AiVersions | undefined): DropdownOption[] {
  return (data?.versions ?? []).slice(0, VERSION_PICKER_LIMIT).map((entry) => ({
    value: entry.version,
    label: versionLabel(entry),
  }));
}

export function selectedVersion(status: AiStatus | undefined): string {
  return status?.pinnedVersion ?? status?.assetsVersion ?? "";
}
