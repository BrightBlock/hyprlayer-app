import { describe, expect, it } from "vitest";

import { parsePlanDoc, phaseRows, toggleTask } from "./planDoc";

const PATH = "/t/repos/app/shared/plans/2026-09-24-og-images.md";

const PLAN = `---
title: 2026-09-24-og-images
type: plan
status: draft
author: jt
---

# OG images route

## Overview

Serve generated OG images from a route.

- [ ] not a task: before the first phase

## Phase 1: Scaffold

### Changes Required:

#### 1. Route

\`\`\`md
- [ ] fenced, not a task
\`\`\`

### Success Criteria:

#### Automated Verification:
- [x] Types check: \`pnpm typecheck\`
- [ ] Lint passes <!-- verified live -->

---

## Phase 2: Build

- [ ] Port OG Images.html to the route
  continued on a second line
* [X] Wire the image slot
+ [~] Partially there
    - [ ] Nested item

## Testing Strategy

- [ ] not a task: after the last phase
`;

describe("parsePlanDoc", () => {
  it("reads title, status, author and overview", () => {
    const doc = parsePlanDoc(PLAN, PATH);
    expect(doc.title).toBe("OG images route");
    expect(doc.status).toBe("draft");
    expect(doc.author).toBe("jt");
    expect(doc.overview).toBe("Serve generated OG images from a route.\n\n- [ ] not a task: before the first phase");
  });

  it("falls back to frontmatter title, then filename", () => {
    expect(parsePlanDoc("---\ntitle: From FM\n---\n\n## Phase 1: A\n", PATH).title).toBe("From FM");
    expect(parsePlanDoc("## Phase 1: A\n", PATH).title).toBe("2026-09-24-og-images");
  });

  it("yields no phases for a doc without phase headings", () => {
    expect(parsePlanDoc("# T\n\n- [ ] loose\n", PATH).phases).toEqual([]);
  });

  it("parses phases and ignores checkboxes outside them and inside fences", () => {
    const doc = parsePlanDoc(PLAN, PATH);
    expect(doc.phases.map((p) => [p.label, p.name])).toEqual([
      ["1", "Scaffold"],
      ["2", "Build"],
    ]);
    expect(doc.phases[0].tasks).toEqual([]);
    expect(doc.phases[0].criteria.map((t) => [t.key, t.title, t.checked, t.group])).toEqual([
      ["P1.1", "Types check: pnpm typecheck", true, "automated"],
      ["P1.2", "Lint passes", false, "automated"],
    ]);
  });

  it("handles bullet variants, [~], nested items and continuation lines", () => {
    const tasks = parsePlanDoc(PLAN, PATH).phases[1].tasks;
    expect(tasks.map((t) => [t.key, t.title, t.checked])).toEqual([
      ["P2.1", "Port OG Images.html to the route", false],
      ["P2.2", "Wire the image slot", true],
      ["P2.3", "Partially there", false],
      ["P2.4", "Nested item", false],
    ]);
  });

  it("accepts em-dash, lettered and prefixed phase headings", () => {
    const doc = parsePlanDoc(
      "## Phase 2 — Frontend: tracker\n- [ ] a\n## Phase 4b: split\n- [ ] b\n# BRI-5 Phase 1: core\n- [ ] c\n",
      PATH,
    );
    expect(doc.phases.map((p) => [p.label, p.name])).toEqual([
      ["2", "Frontend: tracker"],
      ["4b", "split"],
      ["1", "core"],
    ]);
    expect(doc.phases[1].tasks[0].key).toBe("P4b.1");
  });

  describe("stable task ids", () => {
    const TAGGED = "## Phase 1: A\n- [ ] [P1.7] Tagged task\n- [ ] Trailing id <!-- id: route-types -->\n- [ ] Untagged\n";

    it("reads a leading [P…] tag or a trailing id comment and strips both from the title", () => {
      const tasks = parsePlanDoc(TAGGED, PATH).phases[0].tasks;
      expect(tasks.map((t) => [t.key, t.title, t.explicitId])).toEqual([
        ["P1.7", "Tagged task", true],
        ["route-types", "Trailing id", true],
        ["P1.3", "Untagged", false],
      ]);
    });

    it("keeps explicit ids when a task is inserted above them", () => {
      const inserted = TAGGED.replace("## Phase 1: A\n", "## Phase 1: A\n- [ ] Inserted\n");
      const keys = parsePlanDoc(inserted, PATH).phases[0].tasks.map((t) => t.key);
      expect(keys).toEqual(["P1.1", "P1.7", "route-types", "P1.4"]);
    });

    it("never emits a duplicate key", () => {
      const clash = "## Phase 1: A\n- [ ] [P1.2] First\n- [ ] Second\n- [ ] [P1.2] Dup\n";
      const keys = parsePlanDoc(clash, PATH).phases[0].tasks.map((t) => t.key);
      expect(new Set(keys).size).toBe(keys.length);
      expect(keys[0]).toBe("P1.2");
    });

    it("toggles by explicit id even after the title changed", () => {
      const renamed = TAGGED.replace("Tagged task", "Tagged task, reworded");
      const next = toggleTask(renamed, { key: "P1.7", title: "Tagged task" }, true)!;
      expect(next).toContain("- [x] [P1.7] Tagged task, reworded");
    });
  });

  it("keeps the body of a phase-less plan, minus frontmatter and title", () => {
    const claude = "# Plan creation flow\n\n## Context\n\nWhy.\n\n### 1. Parser\n- do it\n";
    const doc = parsePlanDoc(claude, "/Users/jt/.claude/plans/ethereal-leaping-dewdrop.md");
    expect(doc.title).toBe("Plan creation flow");
    expect(doc.phases).toEqual([]);
    expect(doc.body).toBe("## Context\n\nWhy.\n\n### 1. Parser\n- do it");
    expect(parsePlanDoc("---\nstatus: draft\n---\n# T\n\nBody\n", PATH).body).toBe("Body");
  });

  it("titles an untitled inline plan 'Plan'", () => {
    expect(parsePlanDoc("1. Do the thing\n", "").title).toBe("Plan");
  });

  it("parses CRLF input the same as LF", () => {
    const crlf = parsePlanDoc(PLAN.replace(/\n/g, "\r\n"), PATH);
    const lf = parsePlanDoc(PLAN, PATH);
    expect(crlf.phases).toEqual(lf.phases);
    expect(crlf.title).toBe(lf.title);
  });
});

describe("success criteria", () => {
  const MIXED = [
    "## Phase 1: A",
    "- [ ] Write it",
    "### Success Criteria:",
    "- [ ] Ungrouped",
    "#### Automated Verification:",
    "- [x] fmt",
    "#### Manual Verification (/verify in the dev app):",
    "- [ ] Looks right",
    "### Notes",
    "- [ ] After the section",
    "## Phase 2: B",
    "- [ ] Next phase task",
    "",
  ].join("\n");

  it("splits checks from tasks, grouped by their Automated and Manual sub-headings", () => {
    const [a, b] = parsePlanDoc(MIXED, PATH).phases;
    expect(a.tasks.map((t) => t.title)).toEqual(["Write it", "After the section"]);
    expect(a.criteria.map((c) => [c.title, c.checked, c.group])).toEqual([
      ["Ungrouped", false, null],
      ["fmt", true, "automated"],
      ["Looks right", false, "manual"],
    ]);
    expect(b.tasks.map((t) => t.title)).toEqual(["Next phase task"]);
    expect(b.criteria).toEqual([]);
  });

  it("keeps positional keys in document order across tasks and checks", () => {
    expect(phaseRows(parsePlanDoc(MIXED, PATH).phases[0]).map((r) => [r.key, r.title])).toEqual([
      ["P1.1", "Write it"],
      ["P1.2", "Ungrouped"],
      ["P1.3", "fmt"],
      ["P1.4", "Looks right"],
      ["P1.5", "After the section"],
    ]);
  });

  it("toggles a check the same way as a task", () => {
    expect(toggleTask(MIXED, { key: "P1.4", title: "Looks right" }, true)).toContain("- [x] Looks right");
  });
});

describe("toggleTask", () => {
  const doc = parsePlanDoc(PLAN, PATH);
  const port = doc.phases[1].tasks[0];

  it("flips exactly one line and leaves every other byte alone", () => {
    const next = toggleTask(PLAN, port, true);
    expect(next).not.toBeNull();
    const before = PLAN.split("\n");
    const after = next!.split("\n");
    expect(after.length).toBe(before.length);
    const changed = after.flatMap((l, i) => (l === before[i] ? [] : [i]));
    expect(changed).toHaveLength(1);
    expect(after[changed[0]]).toBe("- [x] Port OG Images.html to the route");
  });

  it("is idempotent", () => {
    const once = toggleTask(PLAN, port, true)!;
    expect(toggleTask(once, port, true)).toBe(once);
  });

  it("unchecks", () => {
    const wire = doc.phases[1].tasks[1];
    expect(toggleTask(PLAN, wire, false)).toContain("* [ ] Wire the image slot");
  });

  it("preserves CRLF", () => {
    const crlf = PLAN.replace(/\n/g, "\r\n");
    const next = toggleTask(crlf, port, true)!;
    expect(next).toBe(crlf.replace("- [ ] Port OG Images.html", "- [x] Port OG Images.html"));
  });

  it("follows the title when an edit shifted the key", () => {
    const edited = PLAN.replace("## Phase 2: Build\n\n", "## Phase 2: Build\n\n- [ ] Inserted first\n");
    const next = toggleTask(edited, port, true)!;
    expect(next).toContain("- [ ] Inserted first");
    expect(next).toContain("- [x] Port OG Images.html to the route");
  });

  it("refuses when no task carries the title", () => {
    expect(toggleTask(PLAN, { key: "P2.1", title: "renamed away" }, true)).toBeNull();
  });
});
