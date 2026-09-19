// @vitest-environment jsdom
import { afterEach, beforeEach, describe as group, expect, it, vi } from "vitest";

const invoke = vi.fn(() => Promise.resolve());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const { describe, log } = await import("./log");

group("log", () => {
  beforeEach(() => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    invoke.mockClear();
  });
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("writes through the log plugin with the scope as its location", () => {
    log.warn("reconnect", "attempt failed", { code: "server_unavailable", message: "down", remedy: null });
    expect(invoke).toHaveBeenCalledWith("plugin:log|log", {
      level: 4,
      message: "attempt failed: server_unavailable: down",
      location: "reconnect",
    });
  });

  it("writes a repeated line once per window", () => {
    log.error("query", "same failure");
    log.error("query", "same failure");
    log.error("query", "another failure");
    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it("does nothing outside the desktop app", () => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    log.error("test", "no bridge");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("truncates a long message", () => {
    log.info("big", "x".repeat(5000));
    const sent = (invoke.mock.calls[0] as unknown[])[1] as { message: string };
    expect(sent.message.endsWith("… (5000 chars)")).toBe(true);
    expect(sent.message.length).toBeLessThan(2100);
  });
});

group("describe", () => {
  it("formats an AppError as code and message", () => {
    expect(describe({ code: "doc_conflict", message: "changed on disk", remedy: null })).toBe(
      "doc_conflict: changed on disk",
    );
  });

  it("keeps an Error's name, message and stack", () => {
    const text = describe(new TypeError("boom"));
    expect(text.startsWith("TypeError: boom")).toBe(true);
  });
});
