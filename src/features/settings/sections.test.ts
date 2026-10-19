import { describe, expect, it } from "vitest";

import { parseSettingsSection, SETTINGS_SECTIONS } from "./sections";

describe("parseSettingsSection", () => {
  it("keeps every real section", () => {
    for (const section of SETTINGS_SECTIONS) expect(parseSettingsSection(section)).toBe(section);
  });

  it("falls back to general for anything else", () => {
    expect(parseSettingsSection(undefined)).toBe("general");
    expect(parseSettingsSection("")).toBe("general");
    expect(parseSettingsSection("nope")).toBe("general");
    expect(parseSettingsSection(3)).toBe("general");
  });

  it("has a goals section after notifications", () => {
    expect(parseSettingsSection("goals")).toBe("goals");
    expect(SETTINGS_SECTIONS.indexOf("goals")).toBe(SETTINGS_SECTIONS.indexOf("notifications") + 1);
  });

  it("treats workshop as a link out, not a section", () => {
    expect(parseSettingsSection("workshop")).toBe("general");
  });
});
