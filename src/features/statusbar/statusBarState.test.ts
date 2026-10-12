import { describe, expect, it } from "vitest";

import type { ConnectionStatus, Host } from "@/lib/bindings";
import type { ReconnectSnapshot } from "@/features/hosts/reconnectState";
import { initials } from "@/lib/format";
import {
  meterClasses,
  meterPercent,
  meterState,
  syncLabel,
  type MeterState,
} from "./statusBarState";

const NOW = Date.UTC(2026, 8, 24, 12, 0, 0);
const NEXUS: Host = { destination: "nexus", label: "Nexus" };
const IDLE: ReconnectSnapshot = { lostAt: null, attempting: false, halted: null, keyChanged: null };

function status(patch: Partial<ConnectionStatus> = {}): ConnectionStatus {
  return {
    state: "connected",
    host: null,
    serverVersion: 3,
    serverRelease: null,
    serverOutdated: false,
    transcriptSync: false,
    error: null,
    ...patch,
  };
}

const lost = (patch: Partial<ConnectionStatus> = {}) =>
  status({
    state: "lost",
    host: NEXUS,
    error: { code: "server_disconnected", message: "The connection ended.", remedy: null },
    ...patch,
  });
const failed = (patch: Partial<ConnectionStatus> = {}) =>
  status({
    state: "failed",
    host: NEXUS,
    serverVersion: null,
    error: { code: "remote_unreachable", message: "Could not reach nexus.", remedy: null },
    ...patch,
  });
const droppedAgo = (ms: number): ReconnectSnapshot => ({ ...IDLE, lostAt: NOW - ms });

describe("meterPercent", () => {
  it("floors, so a meter never reaches a threshold early", () => {
    expect(meterPercent(179_999, 200_000)).toBe(89);
    expect(meterPercent(180_000, 200_000)).toBe(90);
    expect(meterPercent(199_999, 200_000)).toBe(99);
  });

  it("clamps overflow to 100 and a negative count to 0", () => {
    expect(meterPercent(250_000, 200_000)).toBe(100);
    expect(meterPercent(-1, 200_000)).toBe(0);
  });

  it("has no percent without a real window, rather than a fake 0%", () => {
    expect(meterPercent(1_000, 0)).toBeNull();
    expect(meterPercent(1_000, -5)).toBeNull();
    expect(meterPercent(1_000, Number.NaN)).toBeNull();
    expect(meterPercent(1_000, Number.POSITIVE_INFINITY)).toBeNull();
    expect(meterPercent(Number.NaN, 200_000)).toBeNull();
    expect(meterPercent(Number.POSITIVE_INFINITY, 200_000)).toBeNull();
  });
});

describe("meterState", () => {
  it("is normal below 90", () => {
    expect(meterState(0)).toBe("normal");
    expect(meterState(50)).toBe("normal");
    expect(meterState(89)).toBe("normal");
  });

  it("warns from 90 inclusive", () => {
    expect(meterState(90)).toBe("warn");
    expect(meterState(99)).toBe("warn");
  });

  it("is full at 100", () => {
    expect(meterState(100)).toBe("full");
  });

  it("never paints a limit as an error", () => {
    const states: MeterState[] = ["normal", "warn", "full"];
    for (const state of states) {
      const { fill, text } = meterClasses(state);
      expect(`${fill} ${text}`).not.toMatch(/\bbad\b|-bad/);
    }
  });
});

describe("syncLabel", () => {
  it("renders nothing until the status resolves", () => {
    expect(syncLabel(null, IDLE, NOW)).toBeNull();
  });

  it("reads 'connected' with a green dot on this machine", () => {
    expect(syncLabel(status(), IDLE, NOW)).toEqual({
      text: "connected",
      title: "Server: this machine",
      tone: "ok",
      ticking: false,
    });
  });

  it("names a remote host in the label", () => {
    expect(syncLabel(status({ host: NEXUS }), IDLE, NOW)).toMatchObject({
      text: "connected · Nexus",
      title: "Server: Nexus",
      tone: "ok",
    });
  });

  it("names another local profile's server in the label", () => {
    expect(syncLabel(status({ localServerName: "staging" }), IDLE, NOW)).toMatchObject({
      text: "connected · staging",
      title: "Server: staging",
      tone: "ok",
    });
  });

  it("falls back to the destination when a remote host has no label", () => {
    const host: Host = { destination: "jt@nexus.local", label: "" };
    expect(syncLabel(status({ host }), IDLE, NOW)?.title).toBe("Server: jt@nexus.local");
  });

  it("keeps reading 'connected' whatever version the server reports", () => {
    expect(syncLabel(status({ serverRelease: "0.8.11+dev.2e671fb" }), IDLE, NOW)?.text).toBe("connected");
  });

  it("warns about an outdated server while still connected", () => {
    expect(syncLabel(status({ host: NEXUS, serverOutdated: true }), IDLE, NOW)).toMatchObject({
      text: "connected · Nexus",
      title: "Server update available on Nexus",
      tone: "warn",
    });
    expect(syncLabel(status({ serverOutdated: true }), IDLE, NOW)?.title).toBe(
      "Server update available on this machine",
    );
  });

  it("reads 'updating' through an update's planned drop, never 'offline' or 'retrying'", () => {
    const updating = {
      text: "updating",
      title: "Updating the server on Nexus",
      tone: "warn",
      ticking: false,
    };
    expect(syncLabel(status({ host: NEXUS, serverOutdated: true }), IDLE, NOW, true)).toEqual(updating);
    expect(syncLabel(lost(), droppedAgo(9_000), NOW, true)).toEqual(updating);
    expect(syncLabel(lost(), { ...droppedAgo(9_000), attempting: true }, NOW, true)).toEqual(updating);
  });

  it("counts up from the drop while lost", () => {
    expect(syncLabel(lost(), droppedAgo(9_000), NOW)).toEqual({
      text: "offline · 9s",
      title: "Connection to Nexus lost",
      tone: "warn",
      ticking: true,
    });
    expect(syncLabel(lost(), droppedAgo(60_000), NOW)?.text).toBe("offline · 1m");
    expect(syncLabel(lost(), droppedAgo(2 * 3_600_000), NOW)?.text).toBe("offline · 2h");
  });

  it("floors the counter at each unit boundary", () => {
    expect(syncLabel(lost(), droppedAgo(59_999), NOW)?.text).toBe("offline · 59s");
    expect(syncLabel(lost(), droppedAgo(60_000), NOW)?.text).toBe("offline · 1m");
    expect(syncLabel(lost(), droppedAgo(3_600_000), NOW)?.text).toBe("offline · 1h");
  });

  it("clamps a drop time from the future to 0s", () => {
    expect(syncLabel(lost(), { ...IDLE, lostAt: NOW + 5_000 }, NOW)?.text).toBe("offline · 0s");
  });

  it("reads plain 'offline' when lost without a known drop time", () => {
    expect(syncLabel(lost(), IDLE, NOW)).toMatchObject({ text: "offline", ticking: false });
  });

  it("reads 'offline' with no counter when never connected", () => {
    expect(syncLabel(failed(), droppedAgo(9_000), NOW)).toEqual({
      text: "offline",
      title: "Not connected to Nexus",
      tone: "bad",
      ticking: false,
    });
  });

  it("reads 'retrying' while an attempt is in flight, in the state's tone", () => {
    const attempting = { ...droppedAgo(9_000), attempting: true };
    expect(syncLabel(lost(), attempting, NOW)).toMatchObject({
      text: "retrying",
      title: "Connection to Nexus lost",
      tone: "warn",
      ticking: false,
    });
    expect(syncLabel(failed(), attempting, NOW)).toMatchObject({
      text: "retrying",
      title: "Not connected to Nexus",
      tone: "bad",
    });
  });

  it("never reads 'retrying' once halted, and says why it stopped", () => {
    const halted = { ...droppedAgo(9_000), halted: "protocol" as const, attempting: true };
    const label = syncLabel(lost(), halted, NOW);
    expect(label?.text).toBe("offline · 9s");
    expect(label?.title).toBe(
      "Connection to Nexus lost · stopped retrying (protocol mismatch)",
    );
  });

  it("lets a live connection win over a stale reconnect state", () => {
    const stale = { ...IDLE, lostAt: NOW - 9_000, attempting: true, halted: "protocol" as const };
    expect(syncLabel(status({ host: NEXUS }), stale, NOW)?.text).toBe("connected · Nexus");
  });

  it("never shows a latency or a paused state it has no data for", () => {
    const statuses = [status(), status({ host: NEXUS, serverOutdated: true }), lost(), failed()];
    const snapshots: ReconnectSnapshot[] = [
      IDLE,
      droppedAgo(1),
      droppedAgo(45_000),
      droppedAgo(5 * 60_000),
      { ...droppedAgo(9_000), attempting: true },
      { ...droppedAgo(9_000), halted: "protocol" },
    ];
    for (const s of statuses) {
      for (const r of snapshots) {
        const text = syncLabel(s, r, NOW)?.text ?? "";
        expect(text).not.toMatch(/ms\b/);
        expect(text).not.toContain("paused");
      }
    }
  });
});

describe("initials", () => {
  it("uppercases a two-letter handle", () => {
    expect(initials("jt")).toBe("JT");
  });

  it("takes one letter per word across the usual separators", () => {
    for (const handle of ["jon tsiros", "jon.tsiros", "jon_tsiros", "jon-tsiros"]) {
      expect(initials(handle)).toBe("JT");
    }
  });

  it("takes the first two letters of a single word", () => {
    expect(initials("jonathan")).toBe("JO");
  });

  it("has none for a blank or missing handle", () => {
    expect(initials("")).toBeNull();
    expect(initials("   ")).toBeNull();
    expect(initials(null)).toBeNull();
    expect(initials(undefined)).toBeNull();
  });
});
