import { describe, expect, it } from "vitest";

import {
  engineMintsViaExec,
  engineResumesById,
  mintPtySessionId,
  relaunchArgs,
  type ResolvedEngine,
} from "./engines";

const eng = (id: string, args: string[] = []): ResolvedEngine => ({
  id,
  label: id,
  glyph: "▸",
  accentToken: "--color-accent",
  command: id,
  args,
  enabled: true,
  builtin: true,
});

const ID = "11111111-1111-4111-8111-111111111111";

describe("relaunchArgs — assign-mode engines (Claude, Gemini)", () => {
  it("Claude pins the id fresh and resumes it by id", () => {
    const claude = eng("claude-code");
    expect(relaunchArgs(claude, { sessionId: ID, resume: false, bypass: false })).toEqual([
      "--session-id",
      ID,
    ]);
    expect(relaunchArgs(claude, { sessionId: ID, resume: true, bypass: false })).toEqual([
      "--resume",
      ID,
    ]);
  });

  it("Claude falls back to --continue when resuming without an id", () => {
    expect(relaunchArgs(eng("claude-code"), { resume: true, bypass: false })).toEqual(["--continue"]);
  });

  it("Gemini uses --session-id / --resume <id> / --resume latest", () => {
    const gemini = eng("gemini");
    expect(relaunchArgs(gemini, { sessionId: ID, resume: false, bypass: false })).toEqual([
      "--session-id",
      ID,
    ]);
    expect(relaunchArgs(gemini, { sessionId: ID, resume: true, bypass: false })).toEqual([
      "--resume",
      ID,
    ]);
    expect(relaunchArgs(gemini, { resume: true, bypass: false })).toEqual(["--resume", "latest"]);
  });

  it("appends the bypass flag after the session args", () => {
    expect(relaunchArgs(eng("claude-code"), { sessionId: ID, resume: true, bypass: true })).toEqual([
      "--resume",
      ID,
      "--dangerously-skip-permissions",
    ]);
    expect(relaunchArgs(eng("gemini"), { resume: false, bypass: true })).toEqual(["--yolo"]);
  });
});

describe("relaunchArgs — exec-mint (Codex) + capture (Antigravity)", () => {
  it("Codex resumes its (minted) id via the `resume` subcommand, fresh or resume", () => {
    const codex = eng("codex");
    // The id is minted before launch, so even a "fresh" launch resumes it.
    expect(relaunchArgs(codex, { sessionId: ID, resume: false, bypass: false })).toEqual([
      "resume",
      ID,
    ]);
    expect(relaunchArgs(codex, { sessionId: ID, resume: true, bypass: false })).toEqual([
      "resume",
      ID,
    ]);
    // Bypass goes after the resume subcommand (codex accepts it there).
    expect(relaunchArgs(codex, { sessionId: ID, resume: true, bypass: true })).toEqual([
      "resume",
      ID,
      "--dangerously-bypass-approvals-and-sandbox",
    ]);
  });

  it("Codex without an id starts bare (fresh) or resumes most-recent", () => {
    // Mint failed / not yet minted: a fresh launch is bare codex; the on-disk
    // capture then backfills the id.
    expect(relaunchArgs(eng("codex"), { resume: false, bypass: false })).toEqual([]);
    expect(relaunchArgs(eng("codex"), { resume: true, bypass: false })).toEqual(["resume", "--last"]);
  });

  it("Antigravity captures its id: bare fresh, resume by --conversation <id>", () => {
    const agy = eng("antigravity");
    // No caller-set id: fresh launch is bare; the id is captured from disk after.
    expect(relaunchArgs(agy, { sessionId: ID, resume: false, bypass: false })).toEqual([]);
    expect(relaunchArgs(agy, { sessionId: ID, resume: true, bypass: false })).toEqual([
      "--conversation",
      ID,
    ]);
    // No id yet: fall back to most-recent.
    expect(relaunchArgs(agy, { resume: true, bypass: false })).toEqual(["--continue"]);
  });
});

describe("relaunchArgs — base args + unknown engines", () => {
  it("prepends the engine's own base args", () => {
    expect(
      relaunchArgs(eng("claude-code", ["--mcp-config", "x"]), { sessionId: ID, resume: true, bypass: false }),
    ).toEqual(["--mcp-config", "x", "--resume", ID]);
  });

  it("a custom engine has no session or bypass args", () => {
    const custom = eng("my-cli", ["--foo"]);
    expect(relaunchArgs(custom, { sessionId: ID, resume: true, bypass: true })).toEqual(["--foo"]);
    expect(relaunchArgs(custom, { resume: false, bypass: false })).toEqual(["--foo"]);
  });
});

describe("session identity helpers", () => {
  it("mints a local id only for assign-mode engines", () => {
    expect(mintPtySessionId("claude-code")).toMatch(/[0-9a-f-]{36}/i);
    expect(mintPtySessionId("gemini")).toMatch(/[0-9a-f-]{36}/i);
    // Exec-mint (codex) mints via a probe; capture (antigravity) + custom have no id.
    expect(mintPtySessionId("codex")).toBeUndefined();
    expect(mintPtySessionId("antigravity")).toBeUndefined();
    expect(mintPtySessionId("my-cli")).toBeUndefined();
  });

  it("flags codex as exec-mint and which engines can resume by id", () => {
    expect(engineMintsViaExec("codex")).toBe(true);
    expect(engineMintsViaExec("antigravity")).toBe(false);
    expect(engineMintsViaExec("claude-code")).toBe(false);

    // resume-by-id gates the on-disk capture fallback; only a custom engine can't.
    expect(engineResumesById("codex")).toBe(true);
    expect(engineResumesById("antigravity")).toBe(true);
    expect(engineResumesById("claude-code")).toBe(true);
    expect(engineResumesById("gemini")).toBe(true);
    expect(engineResumesById("my-cli")).toBe(false);
  });
});
