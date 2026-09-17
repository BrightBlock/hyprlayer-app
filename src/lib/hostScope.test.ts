// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";

const SEVEN = [
  "hyprlayer:last-route",
  "hyprlayer:workstream-tabs",
  "hyprlayer:written-docs",
  "hyprlayer:scripts-dock",
  "hyprlayer:pinned-workstreams",
  "hyprlayer:library-scope",
  "hyprlayer:project-tree",
];

// PROFILE_ID is read once at module evaluation, so each profile needs a fresh
// module graph.
async function load(profile: string | undefined) {
  if (profile === undefined) delete window.__HYPRLAYER_PROFILE__;
  else window.__HYPRLAYER_PROFILE__ = profile;
  vi.resetModules();
  return import("./hostScope");
}

function seedUnprefixed() {
  SEVEN.forEach((key, i) => localStorage.setItem(key, `value-${i}`));
}

afterEach(() => {
  localStorage.clear();
  delete window.__HYPRLAYER_PROFILE__;
});

describe("scopedKey", () => {
  it("falls back to the local profile when the init script never ran", async () => {
    const { PROFILE_ID, scopedKey } = await load(undefined);
    expect(PROFILE_ID).toBe("local");
    expect(scopedKey("hyprlayer:last-route")).toBe("hyprlayer:last-route");
  });

  it("is the identity for the local profile", async () => {
    const { scopedKey } = await load("local");
    expect(SEVEN.map(scopedKey)).toEqual(SEVEN);
  });

  it("prefixes a destination exactly once", async () => {
    const { scopedKey } = await load("nexus");
    expect(scopedKey("hyprlayer:last-route")).toBe("hyprlayer:nexus:last-route");
    expect(scopedKey("plain")).toBe("hyprlayer:nexus:plain");
  });
});

describe("adoptUnprefixedKeys", () => {
  it("moves the seven keys into an empty remote namespace", async () => {
    seedUnprefixed();
    localStorage.setItem("hyprlayer:theme", "dark");

    const { adoptUnprefixedKeys } = await load("nexus");

    SEVEN.forEach((key, i) => {
      expect(localStorage.getItem(`hyprlayer:nexus:${key.slice("hyprlayer:".length)}`)).toBe(
        `value-${i}`,
      );
      expect(localStorage.getItem(key)).toBeNull();
    });
    // Machine-local keys stay where they are.
    expect(localStorage.getItem("hyprlayer:theme")).toBe("dark");

    // A second run has a populated namespace and must not touch it.
    localStorage.setItem("hyprlayer:last-route", "later");
    adoptUnprefixedKeys();
    expect(localStorage.getItem("hyprlayer:nexus:last-route")).toBe("value-0");
    expect(localStorage.getItem("hyprlayer:last-route")).toBe("later");
  });

  it("does nothing on the local profile", async () => {
    seedUnprefixed();

    await load("local");

    SEVEN.forEach((key, i) => expect(localStorage.getItem(key)).toBe(`value-${i}`));
  });

  // The adoption is about the upgrade, not about a profile being new. A user
  // who only ever ran locally and then adds a remote must not have their local
  // tabs walk across with them.
  it("leaves a local profile's keys alone when a remote is adopted later", async () => {
    seedUnprefixed();

    await load("local");
    await load("nexus");

    SEVEN.forEach((key, i) => {
      expect(localStorage.getItem(key)).toBe(`value-${i}`);
      expect(localStorage.getItem(`hyprlayer:nexus:${key.slice("hyprlayer:".length)}`)).toBeNull();
    });
  });
});
