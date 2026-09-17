import { describe, expect, it } from "vitest";

import { validateDestination, validateHostList } from "./hosts";

describe("validateDestination", () => {
  it("accepts the ssh destinations a config file holds", () => {
    expect(validateDestination("nexus")).toBeNull();
    expect(validateDestination("jt@nexus.local")).toBeNull();
    expect(validateDestination("  nexus  ")).toBeNull();
  });

  it("says nothing about a blank destination — the list check owns that", () => {
    expect(validateDestination("")).toBeNull();
    expect(validateDestination("   ")).toBeNull();
  });

  it("refuses whitespace inside a destination", () => {
    expect(validateDestination("jt@nexus local")).toBe("cannot contain spaces");
  });

  it("reserves `local` for this machine's own profile", () => {
    expect(validateDestination("local")).toContain('cannot be "local"');
    expect(validateDestination("  local ")).toContain('cannot be "local"');
    expect(validateDestination("jt@local")).toBeNull();
    expect(validateDestination("localhost")).toBeNull();
  });
});

describe("validateHostList", () => {
  it("passes a well-formed list", () => {
    expect(
      validateHostList([
        { label: "Nexus", destination: "nexus" },
        { label: "Laptop", destination: "jt@laptop.local" },
      ]).size,
    ).toBe(0);
  });

  it("names the row that has no label", () => {
    const errors = validateHostList([{ label: "  ", destination: "nexus" }]);
    expect(errors.get(0)?.label).toContain("Label required");
    expect(errors.get(0)?.destination).toBeUndefined();
  });

  it("names the row that has no destination", () => {
    const errors = validateHostList([{ label: "Nexus", destination: "" }]);
    expect(errors.get(0)?.destination).toContain("SSH destination required");
  });

  it("marks only the second of two rows sharing a destination", () => {
    const errors = validateHostList([
      { label: "Nexus", destination: "nexus" },
      { label: "Nexus again", destination: " nexus " },
    ]);
    expect(errors.has(0)).toBe(false);
    expect(errors.get(1)?.destination).toContain("Already used by another host");
  });

  it("reports the reserved name as a destination error", () => {
    const errors = validateHostList([{ label: "Local", destination: "local" }]);
    expect(errors.get(0)?.destination).toContain('cannot be "local"');
  });

  it("reports a blank label and a bad destination on the same row", () => {
    const errors = validateHostList([{ label: "", destination: "a b" }]);
    expect(errors.get(0)?.label).toContain("Label required");
    expect(errors.get(0)?.destination).toContain("cannot contain spaces");
  });
});
