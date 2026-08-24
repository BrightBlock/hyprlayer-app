import { describe, expect, it } from "vitest";

import type { AiStatus, AiVersions } from "@/lib/bindings";

import {
  bundlePinState,
  formatPublished,
  selectedVersion,
  VERSION_PICKER_LIMIT,
  versionLabel,
  versionOptions,
} from "./bundleVersions";

/** Real `hyprlayer ai status --json` from CLI 1.6.0 (9a2a33c). */
const STATUS: AiStatus = {
  agentTool: "Claude Code",
  agentsInstalledSha: "9a2a33c6fc0648e8cd7bac278365c8a9dcca220b",
  assetsVersion: "1.6.0",
  binaryVersion: "1.6.0",
  installed: true,
  lastAgentCheck: null,
  location: "~/.claude/",
  pinnedVersion: "1.6.0",
};

/** Real `hyprlayer ai versions --json` from the same binary. */
const VERSIONS: AiVersions = {
  assetsVersion: "1.6.0",
  binaryVersion: "1.6.0",
  harness: "claude",
  pinnedVersion: "1.6.0",
  versions: [
    { version: "1.6.0", publishedAt: "2026-08-24T04:13:17Z", current: true, pinned: true },
    { version: "1.6.0-rc.2", publishedAt: "2026-08-24T03:17:01Z", current: false, pinned: false },
    { version: "1.6.0-rc.1", publishedAt: "2026-08-24T01:06:31Z", current: false, pinned: false },
  ],
};

describe("bundlePinState", () => {
  it("reads the pinned state out of a 1.6.0 status", () => {
    expect(bundlePinState(STATUS)).toEqual({
      kind: "pinned",
      installed: "1.6.0",
      binary: "1.6.0",
      pinned: "1.6.0",
      drifted: false,
    });
  });

  it("flags a pin that no longer matches the binary", () => {
    const state = bundlePinState({ ...STATUS, pinnedVersion: "1.5.0", binaryVersion: "1.6.0" });
    expect(state).toMatchObject({ kind: "pinned", pinned: "1.5.0", drifted: true });
  });

  // `pinnedVersion: null` is a real answer, not a missing one. Reading it as
  // anything but "unpinned" would invert the row's headline claim and offer a
  // "Clear pin" button for a pin that does not exist.
  it("reads a null pin as unpinned", () => {
    expect(bundlePinState({ ...STATUS, pinnedVersion: null })).toEqual({
      kind: "unpinned",
      installed: "1.6.0",
      binary: "1.6.0",
    });
  });

  it("reports a pre-1.6.0 CLI as unavailable rather than unpinned", () => {
    expect(
      bundlePinState({
        agentTool: "Claude Code",
        installed: true,
        location: "~/.claude/",
        agentsInstalledSha: "abc",
        lastAgentCheck: null,
      }),
    ).toEqual({ kind: "unavailable" });
  });

  it("reports an unconfigured or still-loading tool as unavailable", () => {
    expect(bundlePinState(undefined)).toEqual({ kind: "unavailable" });
    expect(bundlePinState({ installed: false })).toEqual({ kind: "unavailable" });
  });
});

describe("versionOptions", () => {
  it("builds one option per listed version, in order", () => {
    expect(versionOptions(VERSIONS).map((o) => o.value)).toEqual([
      "1.6.0",
      "1.6.0-rc.2",
      "1.6.0-rc.1",
    ]);
  });

  it("marks the current and pinned entries in their labels", () => {
    const [current, next] = versionOptions(VERSIONS);
    expect(current.label).toContain("installed");
    expect(current.label).toContain("pinned");
    expect(next.label).not.toContain("installed");
    expect(next.label).not.toContain("pinned");
  });

  /**
   * The load-bearing rule. `reinstall --version` accepts any string and, for a
   * version carrying no published bundle, installs master's tree under the
   * requested label instead of failing — so an option sourced from anywhere
   * but `versions` is a corrupted `~/.claude`, not a bad label. The payload
   * below repeats the trap concretely: `binaryVersion` and `pinnedVersion`
   * both name 1.5.9, which the CLI deliberately left out of `versions`.
   */
  it("offers nothing that is absent from the versions array", () => {
    const stale: AiVersions = {
      assetsVersion: "1.5.9",
      binaryVersion: "1.5.9",
      harness: "claude",
      pinnedVersion: "1.5.9",
      versions: [
        { version: "1.6.0", publishedAt: "2026-08-24T04:13:17Z", current: false, pinned: false },
      ],
    };
    const values = versionOptions(stale).map((o) => o.value);
    expect(values).toEqual(["1.6.0"]);
    expect(values).not.toContain("1.5.9");
  });

  it("offers nothing when no release carries a bundle", () => {
    expect(versionOptions({ ...VERSIONS, versions: [] })).toEqual([]);
  });

  // The CLI lists newest-first, so the cap keeps the head: the current
  // release and the rollback candidates, not an archive of old bundles each
  // able to park a machine behind a drifting pin.
  it("offers at most the newest three versions", () => {
    const crowded: AiVersions = {
      ...VERSIONS,
      versions: [
        { version: "1.6.1", publishedAt: "2026-08-25T09:00:00Z", current: false, pinned: false },
        ...(VERSIONS.versions ?? []),
        { version: "1.5.9", publishedAt: "2026-08-20T12:00:00Z", current: false, pinned: false },
      ],
    };
    const values = versionOptions(crowded).map((o) => o.value);
    expect(values).toHaveLength(VERSION_PICKER_LIMIT);
    expect(values).toEqual(["1.6.1", "1.6.0", "1.6.0-rc.2"]);
    expect(values).not.toContain("1.5.9");
  });

  // The list query is `enabled`-gated and can be offline-failed; both arrive
  // as `undefined`, and neither may leave a stale option behind.
  it("offers nothing before the list has loaded", () => {
    expect(versionOptions(undefined)).toEqual([]);
  });
});

describe("selectedVersion", () => {
  it("names the pin when one is set", () => {
    expect(selectedVersion(STATUS)).toBe("1.6.0");
  });

  it("falls back to the installed version when nothing is pinned", () => {
    expect(selectedVersion({ ...STATUS, pinnedVersion: null })).toBe("1.6.0");
  });

  /**
   * A pin whose release was deleted, or one set by hand outside the app. The
   * row still has to name it — that is the state the user is in, and blanking
   * it would hide the only thing that explains their bundle. Naming is not
   * offering: `versionOptions` still refuses to make it selectable.
   */
  it("names a pinned version that is absent from the list, without offering it", () => {
    expect(selectedVersion({ ...STATUS, pinnedVersion: "1.5.9" })).toBe("1.5.9");
    expect(versionOptions(VERSIONS).map((o) => o.value)).not.toContain("1.5.9");
  });

  // Offline with a cold CLI cache: the list never arrives, but `ai status` is
  // local and the row's headline must survive it.
  it("names the installed version with no list loaded at all", () => {
    expect(selectedVersion(STATUS)).toBe("1.6.0");
    expect(versionOptions(undefined)).toEqual([]);
  });

  it("is empty only when the CLI reports no versions at all", () => {
    expect(selectedVersion({ installed: true })).toBe("");
    expect(selectedVersion(undefined)).toBe("");
  });
});

describe("versionLabel", () => {
  it("keeps a bare version when the release has no date or flags", () => {
    expect(versionLabel({ version: "1.6.0" })).toBe("1.6.0");
  });

  it("survives an unparseable published date", () => {
    expect(versionLabel({ version: "1.6.0", publishedAt: "not-a-date" })).toBe("1.6.0");
    expect(formatPublished("not-a-date")).toBeNull();
    expect(formatPublished(null)).toBeNull();
  });
});
