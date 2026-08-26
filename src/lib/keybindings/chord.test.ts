import { describe, expect, it } from "vitest";

import { eventToChord, formatChord, hasPrimaryModifier, normalizeChord } from "./chord";

type KeyParts = Parameters<typeof eventToChord>[0];

function key(over: Partial<KeyParts> & { key: string }): KeyParts {
  return { metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...over };
}

describe("eventToChord", () => {
  it("maps Cmd to mod on macOS and Ctrl to mod elsewhere", () => {
    expect(eventToChord(key({ key: "k", metaKey: true }), true)).toBe("mod+k");
    expect(eventToChord(key({ key: "k", ctrlKey: true }), false)).toBe("mod+k");
  });

  it("keeps the non-primary Cmd/Ctrl key as its literal token", () => {
    expect(eventToChord(key({ key: "k", ctrlKey: true }), true)).toBe("ctrl+k");
    expect(eventToChord(key({ key: "k", metaKey: true }), false)).toBe("meta+k");
  });

  it("orders modifiers canonically", () => {
    const e = key({ key: "k", metaKey: true, altKey: true, shiftKey: true });
    expect(eventToChord(e, true)).toBe("mod+alt+shift+k");
  });

  it("returns null for a bare modifier press", () => {
    expect(eventToChord(key({ key: "Shift", shiftKey: true }), true)).toBeNull();
    expect(eventToChord(key({ key: "Meta", metaKey: true }), true)).toBeNull();
    expect(eventToChord(key({ key: "Control", ctrlKey: true }), false)).toBeNull();
  });

  it("normalizes letters, digits, and punctuation to stable tokens", () => {
    expect(eventToChord(key({ key: "K", metaKey: true, shiftKey: true }), true)).toBe(
      "mod+shift+k",
    );
    expect(eventToChord(key({ key: "1", metaKey: true }), true)).toBe("mod+1");
    expect(eventToChord(key({ key: ",", metaKey: true }), true)).toBe("mod+comma");
    expect(eventToChord(key({ key: "[", metaKey: true }), true)).toBe("mod+bracketleft");
    expect(eventToChord(key({ key: "\\", metaKey: true }), true)).toBe("mod+backslash");
    expect(eventToChord(key({ key: "Enter", metaKey: true, shiftKey: true }), true)).toBe(
      "mod+shift+enter",
    );
  });

  it("folds a shifted character back onto its unshifted key token", () => {
    expect(eventToChord(key({ key: "}", metaKey: true, shiftKey: true }), true)).toBe(
      "mod+shift+bracketright",
    );
    expect(eventToChord(key({ key: "{", metaKey: true, shiftKey: true }), true)).toBe(
      "mod+shift+bracketleft",
    );
    expect(eventToChord(key({ key: "~", metaKey: true, shiftKey: true }), true)).toBe(
      "mod+shift+backquote",
    );
    expect(eventToChord(key({ key: "#", metaKey: true, shiftKey: true }), true)).toBe(
      "mod+shift+3",
    );
  });

  it("keeps the unshifted chord distinct from its shifted twin", () => {
    expect(eventToChord(key({ key: "]", metaKey: true }), true)).toBe("mod+bracketright");
    expect(eventToChord(key({ key: "=", metaKey: true }), true)).toBe("mod+equal");
    expect(eventToChord(key({ key: "+", metaKey: true, shiftKey: true }), true)).toBe(
      "mod+shift+equal",
    );
  });
});

describe("normalizeChord", () => {
  it("lowercases and orders modifiers", () => {
    expect(normalizeChord("Shift+Mod+K")).toBe("mod+shift+k");
    expect(normalizeChord("ALT+MOD+COMMA")).toBe("mod+alt+comma");
  });

  it("is idempotent", () => {
    const once = normalizeChord("shift+mod+k");
    expect(normalizeChord(once)).toBe(once);
  });

  it("tolerates whitespace and stray separators", () => {
    expect(normalizeChord(" mod + shift + k ")).toBe("mod+shift+k");
  });
});

describe("formatChord", () => {
  it("renders concatenated glyphs on macOS", () => {
    expect(formatChord("mod+shift+k", true)).toBe("⌘⇧K");
    expect(formatChord("mod+comma", true)).toBe("⌘,");
    expect(formatChord("mod+shift+enter", true)).toBe("⌘⇧↩");
    expect(formatChord("mod+backslash", true)).toBe("⌘\\");
  });

  it("renders +-joined labels on Windows/Linux", () => {
    expect(formatChord("mod+shift+k", false)).toBe("Ctrl+Shift+K");
    expect(formatChord("mod+comma", false)).toBe("Ctrl+,");
    expect(formatChord("mod+period", false)).toBe("Ctrl+.");
  });

  it("returns empty string for an empty chord", () => {
    expect(formatChord("", true)).toBe("");
    expect(formatChord("", false)).toBe("");
  });

  it("round-trips an event through normalize and format", () => {
    const chord = eventToChord(key({ key: "B", metaKey: true, shiftKey: true }), true);
    expect(chord).toBe("mod+shift+b");
    expect(formatChord(normalizeChord(chord!), true)).toBe("⌘⇧B");
  });
});

describe("hasPrimaryModifier", () => {
  it("is true for mod/alt/ctrl/meta chords and false for bare or shift-only", () => {
    expect(hasPrimaryModifier("mod+k")).toBe(true);
    expect(hasPrimaryModifier("alt+k")).toBe(true);
    expect(hasPrimaryModifier("shift+k")).toBe(false);
    expect(hasPrimaryModifier("k")).toBe(false);
  });
});
