/**
 * Agent-bundle version selection: what the AI Agents card says about the pin,
 * and what its picker is allowed to offer.
 *
 * Pure, so the one rule that matters here is testable without a webview: a
 * picker option may only ever come from `ai versions --json`'s `versions`
 * array. `reinstall --version` accepts any string and, for a version with no
 * verifiable bundle, installs master's tree under the requested label instead
 * of failing — so a version assembled from `binaryVersion`, from a release
 * list fetched elsewhere, or typed by the user is not a worse UX, it is a
 * corrupted `~/.claude` that reinstalling the right version does not undo.
 */
import type { AiStatus, AiVersionEntry, AiVersions } from "@/lib/bindings";
import type { DropdownOption } from "@/components/ui";

/** What the card can say about the installed bundle and its pin. */
export type BundlePinState =
  /** The CLI predates 1.6.0 and reports no version fields at all. The row has
   *  nothing to show, and — more to the point — nothing to offer: `ai
   *  versions` and `reinstall --version` do not exist on that binary. */
  | { kind: "unavailable" }
  /** No pin: the bundle follows the `hyprlayer` binary. */
  | { kind: "unpinned"; installed: string | null; binary: string | null }
  | {
      kind: "pinned";
      installed: string | null;
      binary: string | null;
      pinned: string;
      /** The pin names a different version than the binary ships. Called out
       *  because it is the state users get stuck in: the app keeps installing
       *  the pinned bundle across binary upgrades, silently, forever. */
      drifted: boolean;
    };

/**
 * Read the pin out of `ai status --json`.
 *
 * `pinnedVersion` is the authority on whether a pin exists, and `null` there
 * is a real answer (unpinned) rather than a missing one — the three fields
 * arrive together or not at all, so "no version fields" is read off
 * `assetsVersion`/`binaryVersion` instead of off the pin.
 */
export function bundlePinState(status: AiStatus | undefined): BundlePinState {
  if (!status?.installed) return { kind: "unavailable" };
  const installed = status.assetsVersion ?? null;
  const binary = status.binaryVersion ?? null;
  // Both absent is a pre-1.6.0 binary. Either one present means the CLI
  // speaks versions, and an absent pin genuinely means unpinned.
  if (installed === null && binary === null) return { kind: "unavailable" };
  const pinned = status.pinnedVersion ?? null;
  if (pinned === null) return { kind: "unpinned", installed, binary };
  return { kind: "pinned", installed, binary, pinned, drifted: pinned !== binary };
}

/** `"2026-08-24T04:13:17Z"` → `"24 Aug 2026"`; anything unparseable → `null`. */
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

/** The label for one entry: version, then its date and flags. */
export function versionLabel(entry: AiVersionEntry): string {
  const notes: string[] = [];
  const published = formatPublished(entry.publishedAt);
  if (published) notes.push(published);
  if (entry.current) notes.push("installed");
  if (entry.pinned) notes.push("pinned");
  return notes.length > 0 ? `${entry.version} · ${notes.join(" · ")}` : entry.version;
}

/**
 * How many versions the picker offers: the newest three. Pinning exists for
 * "the release from this morning broke my agents, go back one" — not for
 * archaeology — and every version beyond the rollback pair is another chance
 * to park a machine on an old bundle behind a silently drifting pin.
 *
 * A display cap, deliberately smaller than the backend's examined window
 * (`VERSION_LIST_LIMIT`, 10 releases): install-time validation re-checks the
 * choice against that wider window, so a displayed option stays installable
 * even if newer releases land between the menu opening and the click.
 */
export const VERSION_PICKER_LIMIT = 3;

/**
 * The picker's options, derived from `versions` and from nothing else.
 *
 * Not merged with the installed or pinned version from anywhere: the CLI has
 * already dropped releases carrying no bundle for the active harness, so a
 * version absent from this array is a version that must not be installable.
 * That includes the currently pinned one — a pin whose release was deleted
 * (or which was set by hand outside the app) is exactly the case where
 * re-offering it reinstalls the fallback.
 *
 * Capped to the head of the array: the CLI lists releases newest-first (the
 * GitHub releases API's order), so the slice keeps the [`VERSION_PICKER_LIMIT`]
 * most recent.
 */
export function versionOptions(data: AiVersions | undefined): DropdownOption[] {
  return (data?.versions ?? []).slice(0, VERSION_PICKER_LIMIT).map((entry) => ({
    value: entry.version,
    label: versionLabel(entry),
  }));
}

/**
 * Which version the row names, and which option the picker shows as selected.
 *
 * The pin comes first: when a pin is set it is what the next install would
 * reproduce, even while the files on disk are something else (a fallback
 * install leaves exactly that mismatch). Falls back to the installed version
 * so an unpinned bundle still names what it is on.
 *
 * Deliberately not filtered against the loaded list. This is the row's
 * headline, and it comes from `ai status`, which is local and always
 * available — blanking it whenever the GitHub-backed list is missing (offline
 * with a cold CLI cache, or a pin whose release was deleted) would hide the
 * one fact the row exists to state. An unlisted version still cannot be
 * *chosen*, because [`versionOptions`] never offers it.
 */
export function selectedVersion(status: AiStatus | undefined): string {
  return status?.pinnedVersion ?? status?.assetsVersion ?? "";
}
