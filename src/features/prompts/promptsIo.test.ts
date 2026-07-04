import { describe, expect, it } from "vitest";

import type { CustomPrompt } from "../../lib/bindings";
import {
  applyImport,
  buildPromptsFile,
  parsePromptsFile,
  planImport,
  type ExportEntry,
  type ImportDecision,
  type LibrarySnapshot,
  type PromptsFile,
} from "./promptsIo";

// A stable id minter so added-prompt ids are deterministic in assertions.
function idMinter() {
  let n = 0;
  return () => `new-${++n}`;
}

/** A library snapshot with two built-ins and the given custom prompts. */
function snapshot(overrides: Partial<LibrarySnapshot> = {}): LibrarySnapshot {
  return {
    builtins: [
      { id: "commit", name: "Commit" },
      { id: "ship", name: "Ship" },
    ],
    custom: [],
    takenChords: new Set(),
    projectPathByName: new Map(),
    ...overrides,
  };
}

describe("buildPromptsFile", () => {
  const entry: ExportEntry = {
    id: "p1",
    name: "Backfill tests",
    builtin: false,
    body: "Write tests for {{changed_files}}.",
    chord: "mod+shift+t",
    overrides: { "/repos/app": "vitest override" },
  };
  const projectName = (path: string) => (path === "/repos/app" ? "app" : path);

  it("round-trips through parse with chords and overrides on", () => {
    const text = buildPromptsFile([entry], {
      includeChords: true,
      includeOverrides: true,
      projectName,
    });
    const parsed = parsePromptsFile(text);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.file.prompts).toHaveLength(1);
    expect(parsed.file.prompts[0]).toEqual({
      name: "Backfill tests",
      body: "Write tests for {{changed_files}}.",
      chord: "mod+shift+t",
      overrides: { app: "vitest override" }, // keyed by project NAME, not path
    });
  });

  it("omits chords when includeChords is off", () => {
    const text = buildPromptsFile([entry], {
      includeChords: false,
      includeOverrides: true,
      projectName,
    });
    const p = JSON.parse(text).prompts[0];
    expect(p.chord).toBeUndefined();
    expect(p.overrides).toEqual({ app: "vitest override" });
  });

  it("omits overrides when includeOverrides is off", () => {
    const text = buildPromptsFile([entry], {
      includeChords: true,
      includeOverrides: false,
      projectName,
    });
    const p = JSON.parse(text).prompts[0];
    expect(p.overrides).toBeUndefined();
    expect(p.chord).toBe("mod+shift+t");
  });

  it("writes the version and an optional source", () => {
    const text = buildPromptsFile([entry], {
      includeChords: true,
      includeOverrides: false,
      projectName,
      source: "jon",
    });
    const doc = JSON.parse(text);
    expect(doc.hlprompts).toBe(1);
    expect(doc.source).toBe("jon");
  });
});

describe("parsePromptsFile", () => {
  it("rejects non-JSON", () => {
    const r = parsePromptsFile("not json {");
    expect(r.ok).toBe(false);
  });

  it("rejects the wrong shape", () => {
    expect(parsePromptsFile(JSON.stringify({ nope: true })).ok).toBe(false);
    expect(parsePromptsFile(JSON.stringify({ prompts: "x" })).ok).toBe(false);
  });

  it("rejects a newer format version", () => {
    const r = parsePromptsFile(JSON.stringify({ hlprompts: 99, prompts: [{ name: "X", body: "b" }] }));
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toMatch(/newer version/);
  });

  it("drops entries with no name and tolerates missing bodies", () => {
    const r = parsePromptsFile(
      JSON.stringify({
        hlprompts: 1,
        prompts: [{ body: "no name" }, { name: "  Kept  " }],
      }),
    );
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.file.prompts).toEqual([{ name: "Kept", body: "" }]);
  });

  it("errors when there is nothing importable", () => {
    const r = parsePromptsFile(JSON.stringify({ hlprompts: 1, prompts: [{ body: "x" }] }));
    expect(r.ok).toBe(false);
  });
});

describe("planImport", () => {
  const file: PromptsFile = {
    hlprompts: 1,
    prompts: [
      { name: "Fresh", body: "new one" },
      { name: "Commit", body: "clashes with a built-in" },
      { name: "Mine", body: "clashes with a custom", chord: "mod+shift+t" },
      { name: "WithOverride", body: "x", overrides: { app: "a", ghost: "g" } },
    ],
  };
  const snap = snapshot({
    custom: [{ id: "c1", name: "Mine", body: "existing" }],
    takenChords: new Set(["mod+shift+t"]),
    projectPathByName: new Map([["app", "/repos/app"]]),
  });

  it("flags built-in and custom name conflicts, else new", () => {
    const rows = planImport(file, snap);
    expect(rows[0].conflict).toBeNull();
    expect(rows[1].conflict).toEqual({ kind: "builtin" });
    expect(rows[2].conflict).toEqual({ kind: "custom" });
  });

  it("detects a taken chord and unknown override projects", () => {
    const rows = planImport(file, snap);
    expect(rows[2].chordTaken).toBe(true);
    expect(rows[3].unknownProjects).toEqual(["ghost"]); // "app" exists, "ghost" doesn't
  });
});

/** Decisions helper: include all, with a per-key resolution override. */
function decide(
  keys: string[],
  resolutions: Record<string, ImportDecision["resolution"]> = {},
): Record<string, ImportDecision> {
  return Object.fromEntries(
    keys.map((k) => [k, { include: true, resolution: resolutions[k] ?? "keep-both" }]),
  );
}

describe("applyImport", () => {
  it("adds a brand-new prompt with a free chord", () => {
    const file: PromptsFile = { hlprompts: 1, prompts: [{ name: "Fresh", body: "b", chord: "mod+j" }] };
    const snap = snapshot();
    const rows = planImport(file, snap);
    const out = applyImport(rows, decide(["0"]), {}, snap, idMinter());
    expect(out.imported).toBe(1);
    expect(out.firstId).toBe("new-1");
    expect(out.prompts.custom).toEqual([
      { id: "new-1", name: "Fresh", body: "b", overrides: {}, chord: "mod+j", pinned: false },
    ]);
  });

  it("drops a chord that is already taken", () => {
    const file: PromptsFile = { hlprompts: 1, prompts: [{ name: "Fresh", body: "b", chord: "mod+shift+t" }] };
    const snap = snapshot({ takenChords: new Set(["mod+shift+t"]) });
    const out = applyImport(planImport(file, snap), decide(["0"]), {}, snap, idMinter());
    expect(out.prompts.custom?.[0].chord).toBeNull();
  });

  it("gives two imports competing for the same free chord to the first only", () => {
    const file: PromptsFile = {
      hlprompts: 1,
      prompts: [
        { name: "A", body: "a", chord: "mod+k" },
        { name: "B", body: "b", chord: "mod+k" },
      ],
    };
    const snap = snapshot();
    const out = applyImport(planImport(file, snap), decide(["0", "1"]), {}, snap, idMinter());
    expect(out.prompts.custom?.[0].chord).toBe("mod+k");
    expect(out.prompts.custom?.[1].chord).toBeNull();
  });

  it("keep-both suffixes a name that collides", () => {
    const file: PromptsFile = { hlprompts: 1, prompts: [{ name: "Mine", body: "incoming" }] };
    const base = { custom: [{ id: "c1", name: "Mine", body: "existing" }] as CustomPrompt[] };
    const snap = snapshot(base);
    const out = applyImport(planImport(file, snap), decide(["0"]), base, snap, idMinter());
    expect(out.prompts.custom).toHaveLength(2);
    expect(out.prompts.custom?.[1].name).toBe("Mine (imported)");
    expect(out.prompts.custom?.[0].body).toBe("existing"); // original untouched
  });

  it("replace overwrites a custom prompt's body in place", () => {
    const file: PromptsFile = { hlprompts: 1, prompts: [{ name: "Mine", body: "incoming", chord: "mod+m" }] };
    const base = { custom: [{ id: "c1", name: "Mine", body: "old", chord: null }] as CustomPrompt[] };
    const snap = snapshot(base);
    const out = applyImport(planImport(file, snap), decide(["0"], { "0": "replace" }), base, snap, idMinter());
    expect(out.prompts.custom).toHaveLength(1);
    expect(out.prompts.custom?.[0]).toMatchObject({ id: "c1", body: "incoming", chord: "mod+m" });
    expect(out.firstId).toBe("c1");
  });

  it("replace of a built-in writes its builtinBodies override, not a custom", () => {
    const file: PromptsFile = { hlprompts: 1, prompts: [{ name: "Commit", body: "my commit rules" }] };
    const snap = snapshot();
    const out = applyImport(planImport(file, snap), decide(["0"], { "0": "replace" }), {}, snap, idMinter());
    expect(out.prompts.builtinBodies).toEqual({ commit: "my commit rules" });
    expect(out.prompts.custom).toEqual([]);
    expect(out.firstId).toBe("commit");
  });

  it("skip drops a conflicting entry entirely", () => {
    const file: PromptsFile = { hlprompts: 1, prompts: [{ name: "Commit", body: "x" }] };
    const snap = snapshot();
    const out = applyImport(planImport(file, snap), decide(["0"], { "0": "skip" }), {}, snap, idMinter());
    expect(out.imported).toBe(0);
    expect(out.prompts.custom).toEqual([]);
    expect(out.prompts.builtinBodies).toEqual({});
  });

  it("re-homes known overrides by name and drops unknown ones", () => {
    const file: PromptsFile = {
      hlprompts: 1,
      prompts: [{ name: "P", body: "b", overrides: { app: "for app", ghost: "dropped" } }],
    };
    const snap = snapshot({ projectPathByName: new Map([["app", "/repos/app"]]) });
    const out = applyImport(planImport(file, snap), decide(["0"]), {}, snap, idMinter());
    expect(out.prompts.custom?.[0].overrides).toEqual({ "/repos/app": "for app" });
  });

  it("skips unchecked candidates", () => {
    const file: PromptsFile = { hlprompts: 1, prompts: [{ name: "A", body: "a" }, { name: "B", body: "b" }] };
    const snap = snapshot();
    const rows = planImport(file, snap);
    const decisions = { "0": { include: false, resolution: "keep-both" as const }, "1": { include: true, resolution: "keep-both" as const } };
    const out = applyImport(rows, decisions, {}, snap, idMinter());
    expect(out.imported).toBe(1);
    expect(out.prompts.custom?.[0].name).toBe("B");
  });
});
