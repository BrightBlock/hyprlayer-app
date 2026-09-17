import { describe, expect, it } from "vitest";

import type { PromptImportPreview } from "@/lib/bindings";
import {
  buildExportRequest,
  conflictActions,
  exportFileName,
  importRows,
  willImportCount,
  type ExportEntry,
  type ImportDecision,
} from "./promptsIo";

const entry: ExportEntry = {
  id: "p1",
  name: "Backfill tests",
  builtin: false,
  body: "Write tests for {{changed_files}}.",
  chord: "mod+shift+t",
  overrides: { "/repos/app": "vitest override" },
};

function preview(over: Partial<PromptImportPreview> = {}): PromptImportPreview {
  return {
    sourceSha256: "abc",
    sourceVersion: 1,
    source: null,
    prompts: [
      { key: "0", name: "Fresh", body: "a", chord: null, chordTaken: false, unknownProjects: [] },
      { key: "1", name: "Commit", body: "b", chord: null, chordTaken: false, unknownProjects: [] },
      { key: "2", name: "Mine", body: "c", chord: null, chordTaken: false, unknownProjects: [] },
    ],
    conflicts: [
      { key: "1", kind: "builtin" },
      { key: "2", kind: "custom" },
    ],
    ...over,
  };
}

function decide(over: Record<string, Partial<ImportDecision>> = {}) {
  return Object.fromEntries(
    ["0", "1", "2"].map((key) => [
      key,
      { include: true, resolution: "keepBoth", ...over[key] } as ImportDecision,
    ]),
  );
}

describe("exportFileName", () => {
  it("slugifies the prompt name and adds the extension", () => {
    expect(exportFileName("Backfill Tests!")).toBe("backfill-tests.hlprompts");
  });

  it("falls back to a generic name when nothing survives slugification", () => {
    expect(exportFileName("///")).toBe("prompt.hlprompts");
  });
});

describe("buildExportRequest", () => {
  it("maps the dialog's entries and options onto the command payload", () => {
    const request = buildExportRequest([entry], {
      includeChords: true,
      includeOverrides: false,
      source: "jon",
    });
    expect(request).toEqual({
      entries: [
        {
          name: "Backfill tests",
          body: "Write tests for {{changed_files}}.",
          chord: "mod+shift+t",
          overrides: { "/repos/app": "vitest override" },
        },
      ],
      includeChords: true,
      includeOverrides: false,
      source: "jon",
    });
  });

  it("sends a null source when the caller omits one", () => {
    const request = buildExportRequest([entry], {
      includeChords: false,
      includeOverrides: true,
    });
    expect(request.source).toBeNull();
  });
});

describe("importRows", () => {
  it("joins each conflict onto the candidate it belongs to", () => {
    const rows = importRows(preview());
    expect(rows.map((r) => r.conflict)).toEqual([null, "builtin", "custom"]);
    expect(rows[0].name).toBe("Fresh");
  });
});

describe("conflictActions", () => {
  it("sends keepBoth for a candidate with no name conflict", () => {
    const rows = importRows(preview());
    expect(conflictActions(rows, decide())["0"]).toBe("keepBoth");
  });

  it("passes a conflicting candidate's chosen resolution through", () => {
    const rows = importRows(preview());
    const actions = conflictActions(rows, decide({ "1": { resolution: "replace" } }));
    expect(actions["1"]).toBe("replace");
    expect(actions["2"]).toBe("keepBoth");
  });

  it("turns an unchecked candidate into a skip regardless of its resolution", () => {
    const rows = importRows(preview());
    const actions = conflictActions(rows, decide({ "2": { include: false, resolution: "replace" } }));
    expect(actions["2"]).toBe("skip");
  });

  it("defaults a candidate with no decision to a skip", () => {
    const rows = importRows(preview());
    expect(conflictActions(rows, {})).toEqual({ "0": "skip", "1": "skip", "2": "skip" });
  });
});

describe("willImportCount", () => {
  it("counts everything that is not skipped", () => {
    const rows = importRows(preview());
    expect(willImportCount(rows, decide())).toBe(3);
    expect(willImportCount(rows, decide({ "1": { resolution: "skip" } }))).toBe(2);
    expect(willImportCount(rows, decide({ "0": { include: false } }))).toBe(2);
  });
});
