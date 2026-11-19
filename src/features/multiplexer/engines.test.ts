import { describe, expect, it } from "vitest";

import { engineMintsViaExec, engineResumesById } from "./engines";

describe("session identity helpers", () => {
  it("flags codex as exec-mint and which engines can resume by id", () => {
    expect(engineMintsViaExec("codex")).toBe(true);
    expect(engineMintsViaExec("antigravity")).toBe(false);
    expect(engineMintsViaExec("claude-code")).toBe(false);

    expect(engineResumesById("codex")).toBe(true);
    expect(engineResumesById("antigravity")).toBe(true);
    expect(engineResumesById("claude-code")).toBe(true);
    expect(engineResumesById("gemini")).toBe(true);
    expect(engineResumesById("my-cli")).toBe(false);
  });
});
