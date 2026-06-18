import { describe, expect, it } from "vitest";

import { chordToCommand, conflictsFor, effectiveChords, type Overrides } from "./resolve";
import { COMMANDS, defaultChord } from "./commands";
import { isReserved } from "./reserved";
import { normalizeChord } from "./chord";

describe("effectiveChords", () => {
  it("returns the catalog default when there is no override", () => {
    const eff = effectiveChords({}, true);
    expect(eff.get("nav.dashboard")).toBe("mod+1");
    expect(eff.get("session.clear")).toBe("mod+shift+k");
  });

  it("lets an override beat the default", () => {
    const eff = effectiveChords({ "nav.dashboard": "mod+9" }, true);
    expect(eff.get("nav.dashboard")).toBe("mod+9");
  });

  it("treats a null override as explicitly unbound", () => {
    const eff = effectiveChords({ "nav.dashboard": null }, true);
    expect(eff.get("nav.dashboard")).toBeNull();
  });

  it("normalizes override chords", () => {
    const eff = effectiveChords({ "nav.dashboard": "Shift+Mod+J" }, true);
    expect(eff.get("nav.dashboard")).toBe("mod+shift+j");
  });
});

describe("chordToCommand", () => {
  it("indexes effective chords back to command ids", () => {
    const map = chordToCommand({}, true);
    expect(map.get("mod+1")).toBe("nav.dashboard");
    expect(map.get("mod+shift+k")).toBe("session.clear");
  });

  it("excludes unbound commands", () => {
    const overrides: Overrides = { "nav.dashboard": null };
    const map = chordToCommand(overrides, true);
    expect(map.has("mod+1")).toBe(false);
  });
});

describe("conflictsFor", () => {
  it("finds a collision with another command and ignores self", () => {
    // mod+1 is nav.dashboard's default; binding it to nav.library conflicts.
    expect(conflictsFor("mod+1", "nav.library", {}, true)).toEqual(["nav.dashboard"]);
    // Re-asserting nav.dashboard's own default is not a conflict with itself.
    expect(conflictsFor("mod+1", "nav.dashboard", {}, true)).toEqual([]);
  });

  it("returns empty for an unused chord", () => {
    expect(conflictsFor("mod+9", "nav.library", {}, true)).toEqual([]);
  });
});

describe("catalog integrity", () => {
  for (const isMac of [true, false]) {
    const os = isMac ? "macOS" : "other";

    it(`has no internal default collisions on ${os}`, () => {
      const seen = new Map<string, string>();
      for (const cmd of COMMANDS) {
        const chord = defaultChord(cmd, isMac);
        if (!chord) continue;
        expect(
          seen.has(chord),
          `${cmd.id} and ${seen.get(chord)} both default to ${chord} on ${os}`,
        ).toBe(false);
        seen.set(chord, cmd.id);
      }
    });

    it(`uses no system-reserved default on ${os}`, () => {
      for (const cmd of COMMANDS) {
        const chord = defaultChord(cmd, isMac);
        if (!chord) continue;
        expect(isReserved(chord, isMac), `${cmd.id} default ${chord} is reserved on ${os}`).toBe(
          false,
        );
      }
    });

    it(`has unique stable ids and normalized defaults on ${os}`, () => {
      const ids = new Set<string>();
      for (const cmd of COMMANDS) {
        expect(ids.has(cmd.id), `duplicate id ${cmd.id}`).toBe(false);
        ids.add(cmd.id);
        const chord = defaultChord(cmd, isMac);
        if (chord) expect(chord).toBe(normalizeChord(chord));
      }
    });
  }
});
