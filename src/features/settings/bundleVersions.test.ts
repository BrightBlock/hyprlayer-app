import { describe, expect, it } from "vitest";

import type { AiStatus, AiVersions } from "@/lib/bindings";

import {
  bundleLayout,
  bundlePinState,
  cliFailureText,
  formatPublished,
  needsSkillsCliUpdate,
  selectedVersion,
  VERSION_PICKER_LIMIT,
  versionLabel,
  versionOptions,
} from "./bundleVersions";

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

  it("names a pinned version that is absent from the list, without offering it", () => {
    expect(selectedVersion({ ...STATUS, pinnedVersion: "1.5.9" })).toBe("1.5.9");
    expect(versionOptions(VERSIONS).map((o) => o.value)).not.toContain("1.5.9");
  });

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

describe("needsSkillsCliUpdate", () => {
  it("stops nagging a CLI that already clears the floor", () => {
    for (const version of ["1.6.0", "1.6.1", "1.6.2-dev", "1.7.0", "2.0.0", "v1.6.1"]) {
      expect(needsSkillsCliUpdate(version)).toBe(false);
    }
  });

  it("still asks an older CLI to update", () => {
    for (const version of ["1.5.9", "1.5.10", "0.9.0", "1.5"]) {
      expect(needsSkillsCliUpdate(version)).toBe(true);
    }
  });

  it("asks when there is no readable version at all", () => {
    for (const version of [null, undefined, "", "   ", "unknown"]) {
      expect(needsSkillsCliUpdate(version)).toBe(true);
    }
  });

  it("compares each part numerically, not as text", () => {
    expect(needsSkillsCliUpdate("1.10.0")).toBe(false);
    expect(needsSkillsCliUpdate("10.0.0")).toBe(false);
  });
});

describe("cliFailureText", () => {
  // `ai reinstall` names the kept paths on stdout and only a generic error on stderr
  const COLLISION = {
    stdout: "Downloading agent files...\n  Kept your modified settings.json\n",
    stderr: "Error: Claude + Codex agent setup is incomplete because an existing path was left untouched.\n",
  };

  it("keeps the stdout detail that names the collision", () => {
    const text = cliFailureText(COLLISION);
    expect(text).toContain("Kept your modified settings.json");
    expect(text).toContain("setup is incomplete");
  });

  it("reads in terminal order, detail before the error", () => {
    const text = cliFailureText(COLLISION);
    expect(text.indexOf("Kept your modified")).toBeLessThan(text.indexOf("Error:"));
  });

  it("drops an empty stream instead of leaving blank lines", () => {
    expect(cliFailureText({ stdout: "", stderr: "boom\n" })).toBe("boom");
    expect(cliFailureText({ stdout: "  \n", stderr: "" })).toBe("");
  });
});

describe("bundleLayout", () => {
  const BUNDLE = { manageable: true } as never;

  it("calls a CLI too old to report its version a legacy layout", () => {
    expect(bundleLayout({ ...STATUS, binaryVersion: null }, BUNDLE)).toBe("legacy");
  });

  it("calls a current CLI without the pair unpaired, not legacy", () => {
    const status = { ...STATUS, binaryVersion: "1.6.1", assetsVersion: "1.6.1" };
    expect(bundleLayout(status, { manageable: false } as never)).toBe("unpaired");
  });

  it("calls a current CLI with the pair managed", () => {
    expect(bundleLayout({ ...STATUS, binaryVersion: "1.6.2" }, BUNDLE)).toBe("managed");
  });

  it("answers nothing until the freshness probe lands", () => {
    expect(bundleLayout(STATUS, undefined)).toBeNull();
  });
});
