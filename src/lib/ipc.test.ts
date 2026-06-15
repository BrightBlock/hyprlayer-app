import { describe, expect, it } from "vitest";

import { errorMessage, errorRemedy, isAppError, unwrap } from "./ipc";

const ok = <T>(data: T) => Promise.resolve({ status: "ok" as const, data });
const err = (error: { code: string; message: string; remedy: string | null }) =>
  Promise.resolve({ status: "error" as const, error });

describe("unwrap", () => {
  it("returns data on ok", async () => {
    await expect(unwrap(ok(42))).resolves.toBe(42);
  });

  it("throws the typed AppError on error", async () => {
    const e = { code: "binary_missing", message: "claude not found", remedy: "Install it" };
    await expect(unwrap(err(e))).rejects.toEqual(e);
  });
});

describe("error helpers", () => {
  const appError = { code: "x", message: "boom", remedy: "fix it" };

  it("recognizes AppError shapes", () => {
    expect(isAppError(appError)).toBe(true);
    expect(isAppError(new Error("nope"))).toBe(false);
    expect(isAppError(null)).toBe(false);
    expect(isAppError("string")).toBe(false);
  });

  it("extracts messages from anything", () => {
    expect(errorMessage(appError)).toBe("boom");
    expect(errorMessage(new Error("js error"))).toBe("js error");
    expect(errorMessage(7)).toBe("7");
  });

  it("extracts remedies only from AppErrors", () => {
    expect(errorRemedy(appError)).toBe("fix it");
    expect(errorRemedy({ ...appError, remedy: null })).toBeNull();
    expect(errorRemedy(new Error("x"))).toBeNull();
  });
});
