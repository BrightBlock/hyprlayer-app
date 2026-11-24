import type { AgentBundleStatus, AiStatus, AiVersionEntry, AiVersions } from "@/lib/bindings";
import type { DropdownOption } from "@/components/ui";

type BundlePinState =
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

export const SKILLS_CLI_FLOOR = "1.6.0";

function numericVersion(version: string | null | undefined): number[] | null {
  const core = version?.trim().replace(/^v/i, "").split(/[-+\s]/)[0];
  if (!core) return null;
  const parts = core.split(".").map((p) => Number.parseInt(p, 10));
  return parts.some((p) => Number.isNaN(p)) ? null : parts;
}

export function needsSkillsCliUpdate(version: string | null | undefined): boolean {
  const found = numericVersion(version);
  const floor = numericVersion(SKILLS_CLI_FLOOR);
  if (!found || !floor) return true;
  for (let i = 0; i < Math.max(found.length, floor.length); i += 1) {
    const a = found[i] ?? 0;
    const b = floor[i] ?? 0;
    if (a !== b) return a < b;
  }
  return false;
}

type BundleLayout = "managed" | "unpaired" | "legacy";

/**
 * Why the reported install does or doesn't support switching bundle versions.
 * `null` until the freshness probe answers.
 */
export function bundleLayout(
  status: AiStatus | undefined,
  bundle: AgentBundleStatus | undefined,
): BundleLayout | null {
  if (!bundle) return null;
  // a CLI too old to report its own version copied the files in place and cannot relink them
  if (needsSkillsCliUpdate(status?.binaryVersion)) return "legacy";
  return bundle.manageable ? "managed" : "unpaired";
}

/**
 * Both streams of a failed CLI run, in terminal order. The hyprlayer CLI writes the
 * actionable detail (which paths it kept) to stdout and only a generic error to stderr,
 * so showing either one alone loses half the story.
 */
export function cliFailureText(run: { stdout: string; stderr: string }): string {
  return [run.stdout, run.stderr]
    .map((part) => part.trim())
    .filter(Boolean)
    .join("\n\n");
}
