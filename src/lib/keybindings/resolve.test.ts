import { describe, expect, it } from "vitest";

import {
  chordToCommand,
  conflictsFor,
  effectiveAlternates,
  effectiveChords,
  type Overrides,
} from "./resolve";
import { alternateChords, COMMANDS, defaultChord } from "./commands";
import { isReserved } from "./reserved";
import { normalizeChord } from "./chord";

describe("effectiveChords", () => {
  it("returns the catalog default when there is no override", () => {
    const eff = effectiveChords({}, true);
    expect(eff.get("nav.dashboard")).toBe("ctrl+1");
    expect(eff.get("session.clear")).toBe("mod+shift+k");
  });

  it("lets an override beat the default", () => {
    const eff = effectiveChords({ "nav.dashboard": "mod+f6" }, true);
    expect(eff.get("nav.dashboard")).toBe("mod+f6");
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
    expect(map.get("ctrl+1")).toBe("nav.dashboard");
    expect(map.get("mod+shift+k")).toBe("session.clear");
  });

  it("excludes unbound commands", () => {
    const overrides: Overrides = { "nav.dashboard": null };
    const map = chordToCommand(overrides, true);
    expect(map.has("ctrl+1")).toBe(false);
  });

  it("gives the tab chords Safari's meaning", () => {
    const map = chordToCommand({}, true);
    expect(map.get("mod+w")).toBe("tab.archive");
    expect(map.get("mod+shift+w")).toBe("tab.close");
    expect(map.get("mod+t")).toBe("tab.new");
    expect(map.get("mod+shift+t")).toBe("tab.reopen");
    expect(map.get("mod+1")).toBe("tab.select1");
    expect(map.get("mod+8")).toBe("tab.select8");
    expect(map.get("mod+9")).toBe("tab.selectLast");
  });

  it("indexes alternates alongside the primary chord", () => {
    const map = chordToCommand({}, true);
    expect(map.get("ctrl+tab")).toBe("tab.next");
    expect(map.get("mod+shift+bracketright")).toBe("tab.next");
    expect(map.get("ctrl+shift+tab")).toBe("tab.prev");
    expect(map.get("mod+shift+bracketleft")).toBe("tab.prev");
  });

  it("drops a command's alternates once the user overrides it", () => {
    const map = chordToCommand({ "tab.next": "mod+f7" }, true);
    expect(map.get("mod+f7")).toBe("tab.next");
    expect(map.has("mod+shift+bracketright")).toBe(false);
  });

  it("never lets an alternate displace a primary binding", () => {
    const map = chordToCommand({ "tab.prev": "mod+shift+bracketright" }, true);
    expect(map.get("mod+shift+bracketright")).toBe("tab.prev");
  });
});

describe("effectiveAlternates", () => {
  it("returns the catalog alternates, and none once overridden", () => {
    expect(effectiveAlternates({}, "tab.prev", true)).toEqual(["mod+shift+bracketleft"]);
    expect(effectiveAlternates({ "tab.prev": "mod+f7" }, "tab.prev", true)).toEqual([]);
  });

  it("is empty for a command with no alternates and for an unknown id", () => {
    expect(effectiveAlternates({}, "tab.new", true)).toEqual([]);
    expect(effectiveAlternates({}, "nope", true)).toEqual([]);
  });
});

describe("conflictsFor", () => {
  it("finds a collision with another command and ignores self", () => {
    expect(conflictsFor("ctrl+1", "nav.library", {}, true)).toEqual(["nav.dashboard"]);
    expect(conflictsFor("ctrl+1", "nav.dashboard", {}, true)).toEqual([]);
  });

  it("returns empty for an unused chord", () => {
    expect(conflictsFor("mod+f9", "nav.library", {}, true)).toEqual([]);
  });

  it("flags a chord that collides with another command's alternate", () => {
    expect(conflictsFor("mod+shift+bracketleft", "nav.library", {}, true)).toEqual(["tab.prev"]);
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

    it(`keeps alternates normalized, unreserved, and off every other chord on ${os}`, () => {
      const primaries = new Map(
        COMMANDS.flatMap((cmd) => {
          const chord = defaultChord(cmd, isMac);
          return chord ? [[chord, cmd.id] as const] : [];
        }),
      );
      const seen = new Map<string, string>();
      for (const cmd of COMMANDS) {
        for (const alt of alternateChords(cmd, isMac)) {
          expect(alt, `${cmd.id} alternate ${alt} is not normalized`).toBe(normalizeChord(alt));
          expect(isReserved(alt, isMac), `${cmd.id} alternate ${alt} is reserved on ${os}`).toBe(
            false,
          );
          expect(
            primaries.get(alt) ?? null,
            `${cmd.id} alternate ${alt} shadows ${primaries.get(alt)}'s default on ${os}`,
          ).toBe(null);
          expect(
            seen.has(alt),
            `${cmd.id} and ${seen.get(alt)} share the alternate ${alt} on ${os}`,
          ).toBe(false);
          seen.set(alt, cmd.id);
        }
      }
    });
  }
});
