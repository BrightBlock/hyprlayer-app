import { describe, expect, it } from "vitest";

import type { ConnectionStatus, Host, HostsFile } from "@/lib/bindings";
import {
  serverRows,
  serverSummary,
  toHost,
  validateDestination,
  validateHostList,
  type HostProbeResult,
} from "./hosts";

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

  // The destination is ssh's bare argv, so a leading dash is parsed as an option.
  it("refuses a leading dash, which ssh would read as an option", () => {
    expect(validateDestination("-oProxyCommand=x")).toContain('"-"');
    expect(validateDestination("  -p2222")).toContain('"-"');
    expect(validateDestination("--help")).toContain('"-"');
    expect(validateDestination("jt@nexus-2")).toBeNull();
  });

  it("refuses a URL, pointing at the user@host form instead", () => {
    for (const url of ["http://nexus", "https://nexus.local", "ws://nexus:9000", "wss://nexus", "HTTPS://nexus"]) {
      const why = validateDestination(url);
      expect(why, url).toContain("SSH destination");
      expect(why, url).toContain("not a URL");
      expect(why, url).toContain("user@host");
    }
  });

  it("accepts ssh's own URI form", () => {
    expect(validateDestination("ssh://jt@nexus:2222")).toBeNull();
    expect(validateDestination("ssh://nexus")).toBeNull();
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

  it("accepts a row with no label", () => {
    expect(validateHostList([{ label: "  ", destination: "nexus" }]).size).toBe(0);
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

  it("reports a bad destination on a row with no label", () => {
    const errors = validateHostList([{ label: "", destination: "a b" }]);
    expect(errors.get(0)?.label).toBeUndefined();
    expect(errors.get(0)?.destination).toContain("cannot contain spaces");
  });

  it("applies the new destination rules to a row", () => {
    expect(validateHostList([{ label: "", destination: "-oProxyCommand=x" }]).get(0)?.destination).toContain('"-"');
    expect(validateHostList([{ label: "", destination: "https://nexus" }]).get(0)?.destination).toContain(
      "not a URL",
    );
  });
});

describe("toHost", () => {
  it("trims both fields", () => {
    expect(toHost("  jt@nexus.local ", " Nexus  ")).toEqual({ destination: "jt@nexus.local", label: "Nexus" });
  });

  // The backend names a host by its label in every error it raises.
  it("names an unlabelled host after its destination", () => {
    expect(toHost(" nexus ", "")).toEqual({ destination: "nexus", label: "nexus" });
    expect(toHost("nexus", "   ")).toEqual({ destination: "nexus", label: "nexus" });
  });
});

const NEXUS: Host = { destination: "nexus", label: "Nexus" };
const LAPTOP: Host = { destination: "jt@laptop.local", label: "Laptop" };

function file(hosts: Host[], active: string | null = null): HostsFile {
  return { hosts, active };
}

function status(state: ConnectionStatus["state"], host: Host | null, code?: string): ConnectionStatus {
  return {
    state,
    host,
    serverVersion: state === "connected" ? 1 : null,
    serverRelease: null,
    serverOutdated: false,
    error: code ? { code, message: `${code} happened`, remedy: null } : null,
  };
}

const NONE: Record<string, HostProbeResult> = {};

describe("serverRows", () => {
  it("lists only this machine, as the default, when there is no hosts file", () => {
    const rows = serverRows(undefined, undefined, NONE);
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      key: "local",
      host: null,
      name: "This machine",
      isDefault: true,
      isConnected: false,
      state: "unknown",
    });
  });

  it("puts this machine first, then the hosts in file order", () => {
    const rows = serverRows(file([NEXUS, LAPTOP]), undefined, NONE);
    expect(rows.map((r) => r.key)).toEqual(["local", "nexus", "jt@laptop.local"]);
    expect(rows.map((r) => r.name)).toEqual(["This machine", "Nexus", "Laptop"]);
  });

  it("names an unlabelled host after its destination", () => {
    const rows = serverRows(file([{ destination: "nexus", label: "" }]), undefined, NONE);
    expect(rows[1].name).toBe("nexus");
  });

  it("makes the active host the default, and this machine not", () => {
    const rows = serverRows(file([NEXUS, LAPTOP], "jt@laptop.local"), undefined, NONE);
    expect(rows.map((r) => r.isDefault)).toEqual([false, false, true]);
  });

  // Mirrors `hosts.rs` `target`: an active destination naming no listed host counts as none.
  it("falls back to this machine as the default when `active` names no listed host", () => {
    const rows = serverRows(file([NEXUS], "gone"), undefined, NONE);
    expect(rows.map((r) => r.isDefault)).toEqual([true, false]);
  });

  it("takes the connected local row's state from the live connection", () => {
    expect(serverRows(file([NEXUS]), status("connected", null), NONE)[0]).toMatchObject({
      isConnected: true,
      state: "ok",
    });
    expect(serverRows(file([NEXUS]), status("failed", null, "server_unavailable"), NONE)[0]).toMatchObject({
      isConnected: true,
      state: "unreachable",
      failureCode: "server_unavailable",
    });
  });

  it("lets a lost or failed live connection beat an older ok probe", () => {
    const ok = { nexus: { ok: true, at: 1 } };
    const lost = serverRows(file([NEXUS], "nexus"), status("lost", NEXUS, "server_disconnected"), ok);
    expect(lost[1]).toMatchObject({ isConnected: true, state: "unreachable", failureCode: "server_disconnected" });
    const failed = serverRows(file([NEXUS], "nexus"), status("failed", NEXUS, "remote_unreachable"), ok);
    expect(failed[1]).toMatchObject({ isConnected: true, state: "unreachable", failureCode: "remote_unreachable" });
  });

  it("lets a live connection beat an older failed probe", () => {
    const rows = serverRows(file([NEXUS], "nexus"), status("connected", NEXUS), {
      nexus: { ok: false, code: "remote_unreachable", at: 1 },
    });
    expect(rows[1]).toMatchObject({ isConnected: true, state: "ok" });
    expect(rows[0]).toMatchObject({ isConnected: false, state: "unknown" });
  });

  it("takes the other rows' state from this session's probes", () => {
    const rows = serverRows(file([NEXUS, LAPTOP]), status("connected", null), {
      nexus: { ok: true, at: 1 },
      "jt@laptop.local": { ok: false, code: "remote_auth_failed", at: 2 },
    });
    expect(rows[1]).toMatchObject({ isConnected: false, state: "ok" });
    expect(rows[2]).toMatchObject({ isConnected: false, state: "unreachable", failureCode: "remote_auth_failed" });
  });

  it("leaves an untested host unknown rather than unreachable", () => {
    const rows = serverRows(file([NEXUS]), status("connected", null), NONE);
    expect(rows[1].state).toBe("unknown");
    expect(rows[1].failureCode).toBeUndefined();
  });

  it("marks no row connected before the first connection status resolves", () => {
    const rows = serverRows(file([NEXUS], "nexus"), undefined, { nexus: { ok: true, at: 1 } });
    expect(rows.some((r) => r.isConnected)).toBe(false);
    expect(rows[1].state).toBe("ok");
  });
});

describe("serverSummary", () => {
  it("counts every row, and only unreachable ones as unreachable", () => {
    const rows = serverRows(file([NEXUS, LAPTOP, { destination: "box", label: "Box" }]), status("lost", null, "server_disconnected"), {
      nexus: { ok: false, code: "remote_unreachable", at: 1 },
    });
    expect(serverSummary(rows)).toEqual({ total: 4, unreachable: 2 });
  });

  it("does not count untested hosts as unreachable", () => {
    const rows = serverRows(file([NEXUS, LAPTOP]), status("connected", null), NONE);
    expect(serverSummary(rows)).toEqual({ total: 3, unreachable: 0 });
  });
});
