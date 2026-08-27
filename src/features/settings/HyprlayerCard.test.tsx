// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AgentBundleInstall, Bootstrap } from "@/lib/bindings";

import { HyprlayerCard } from "./HyprlayerCard";

const b = vi.hoisted(() => ({
  getAiStatus: vi.fn(),
  agentBundleStatus: vi.fn(),
  agentBundleVersions: vi.fn(),
  detectEditors: vi.fn(),
  hyprlayerCliUpdateCheck: vi.fn(),
  updateHyprlayerCli: vi.fn(),
  installAgentBundleVersion: vi.fn(),
  updateAgentBundle: vi.fn(),
  repairAgentBundle: vi.fn(),
}));
vi.mock("@/lib/bindings", () => ({ commands: b }));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openPath: vi.fn(),
  revealItemInDir: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, ...rest }: { children?: unknown; [k: string]: unknown }) => (
    <a {...rest}>{children as never}</a>
  ),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const BOOTSTRAP = {
  config: {
    state: "ready",
    path: "/home/jt/.config/hyprlayer/config.json",
    user: "jt",
    backendKind: "git",
    configured: true,
  },
  binaries: [
    {
      kind: "hyprlayer",
      name: "hyprlayer",
      path: "/home/jt/.cargo/bin/hyprlayer",
      version: "1.6.2-dev",
      state: { state: "ok" },
      optional: false,
      overridden: false,
    },
  ],
} as unknown as Bootstrap;

const repaired = (over: Partial<AgentBundleInstall> = {}): AgentBundleInstall => ({
  ok: true,
  exitCode: 0,
  source: "fallback",
  stdout: "  Downloaded 73 files from the master repo archive\n",
  stderr: "",
  ...over,
});

function renderCard() {
  b.getAiStatus.mockReturnValue(
    ok({
      agentTool: "Claude + Codex",
      installed: true,
      assetsVersion: "1.6.2-dev",
      binaryVersion: "1.6.2-dev",
      pinnedVersion: null,
      platforms: [
        { id: "claude", name: "Claude Code", installed: true, location: "/home/jt/.claude" },
        { id: "codex", name: "Codex", installed: true, location: "/home/jt/.codex" },
      ],
    }),
  );
  b.agentBundleStatus.mockReturnValue(
    ok({
      manageable: true,
      installedSha: "d705a48aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      latestSha: null,
      installedVersion: "1.6.2-dev",
      desiredVersion: "1.6.2-dev",
      lastChecked: null,
      updateAvailable: false,
    }),
  );
  b.detectEditors.mockReturnValue(ok([]));
  b.hyprlayerCliUpdateCheck.mockReturnValue(ok({ current: "1.6.2-dev", latest: null, hint: null }));

  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <HyprlayerCard data={BOOTSTRAP} />
    </QueryClientProvider>,
  );
}

const repairButton = () => screen.findByRole("button", { name: /repair \/ reinstall/i });

type Rung = {
  status: Record<string, unknown>;
  bundle: Record<string, unknown>;
  cli: Record<string, unknown>;
};

function renderRung({ status, bundle, cli }: Rung) {
  b.getAiStatus.mockReturnValue(ok(status));
  b.agentBundleStatus.mockReturnValue(ok(bundle));
  b.agentBundleVersions.mockReturnValue(ok({ versions: [] }));
  b.detectEditors.mockReturnValue(ok([]));
  b.hyprlayerCliUpdateCheck.mockReturnValue(ok(cli));
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <HyprlayerCard data={BOOTSTRAP} />
    </QueryClientProvider>,
  );
}

const PAIRED = [
  { id: "claude", name: "Claude Code", installed: true, location: "~/.claude" },
  { id: "codex", name: "Codex", installed: true, location: "~/.codex" },
];

const BUNDLE = {
  manageable: true,
  installedSha: "9a2a33c6fc0648e8cd7bac278365c8a9dcca220b",
  latestSha: null,
  installedVersion: "1.6.2",
  desiredVersion: "1.6.2",
  lastChecked: null,
  updateAvailable: false,
};

// The pre-1.6.0 payload captured in hyprcli.rs's `tolerates_a_pre_1_6_0_status_payload`.
const RUNG_1_5_X: Rung = {
  status: {
    agentTool: "Claude Code",
    installed: true,
    location: "~/.claude/",
    agentsInstalledSha: "abc",
    lastAgentCheck: null,
  },
  bundle: { ...BUNDLE, manageable: false, installedVersion: null, desiredVersion: null },
  cli: { current: "1.5.9", latest: "1.6.2", hint: "brew upgrade hyprlayer" },
};

// hyprcli.rs's STATUS_1_6_0: version fields present, Codex not provisioned yet.
const RUNG_1_6_0_UNPAIRED: Rung = {
  status: {
    agentTool: "Claude Code",
    installed: true,
    agentsInstalledSha: "9a2a33c6fc0648e8cd7bac278365c8a9dcca220b",
    assetsVersion: "1.6.0",
    binaryVersion: "1.6.0",
    pinnedVersion: "1.6.0",
    platforms: [],
  },
  bundle: { ...BUNDLE, manageable: false, installedVersion: "1.6.0" },
  cli: { current: "1.6.0", latest: null, hint: null },
};

const RUNG_BUNDLE_BEHIND: Rung = {
  status: {
    agentTool: "Claude + Codex",
    installed: true,
    assetsVersion: "1.5.9",
    binaryVersion: "1.6.2",
    pinnedVersion: null,
    platforms: PAIRED,
  },
  bundle: { ...BUNDLE, installedVersion: "1.5.9", updateAvailable: true },
  cli: { current: "1.6.2", latest: null, hint: null },
};

const RUNG_CURRENT: Rung = {
  status: {
    agentTool: "Claude + Codex",
    installed: true,
    assetsVersion: "1.6.2",
    binaryVersion: "1.6.2",
    pinnedVersion: null,
    platforms: PAIRED,
  },
  bundle: BUNDLE,
  cli: { current: "1.6.2", latest: null, hint: null },
};

describe("the CLI row", () => {
  const cliButton = () => screen.findByRole("button", { name: /update the hyprlayer cli/i });

  it("self-updates through the CLI even when no newer version was reported", async () => {
    b.updateHyprlayerCli.mockReturnValue(ok({ ok: true, exitCode: 0, stdout: "", stderr: "" }));
    renderRung(RUNG_CURRENT);
    fireEvent.click(await cliButton());
    await waitFor(() => expect(b.updateHyprlayerCli).toHaveBeenCalled());
    await waitFor(async () => expect((await cliButton()).className).toContain("text-ok"));
  });

  it("never runs a check as the download action", async () => {
    b.updateHyprlayerCli.mockReturnValue(ok({ ok: true, exitCode: 0, stdout: "", stderr: "" }));
    renderRung(RUNG_CURRENT);
    const before = b.hyprlayerCliUpdateCheck.mock.calls.length;
    fireEvent.click(await cliButton());
    await waitFor(() => expect(b.updateHyprlayerCli).toHaveBeenCalled());
    expect(b.hyprlayerCliUpdateCheck.mock.calls.length).toBeGreaterThanOrEqual(before);
    expect(screen.queryByRole("button", { name: /check for cli updates/i })).toBeNull();
  });

  it("hands over the manual command when the CLI cannot update itself", async () => {
    b.updateHyprlayerCli.mockReturnValue(
      ok({ ok: false, exitCode: 1, stdout: "", stderr: "sudo: no tty present" }),
    );
    renderRung({ ...RUNG_CURRENT, cli: { current: "1.6.2", latest: null, hint: "paru -Syu hyprlayer-bin" } });
    fireEvent.click(await cliButton());
    expect(await screen.findByText(/paru -Syu hyprlayer-bin/)).toBeTruthy();
    expect(screen.getByText(/no tty present/)).toBeTruthy();
    expect((await cliButton()).className).toContain("text-bad");
  });

  it("leaves the action to the migration panel on a legacy install", async () => {
    renderRung(RUNG_1_5_X);
    await screen.findByText(/Legacy layout/);
    expect(screen.queryByRole("button", { name: /update the hyprlayer cli/i })).toBeNull();
  });
});

describe("the 1.5.x to 1.6.2 upgrade ladder", () => {
  it("calls a pre-1.6.0 install a legacy layout and offers the CLI upgrade", async () => {
    renderRung(RUNG_1_5_X);
    expect(await screen.findByText(/Legacy layout/)).toBeTruthy();
    expect(screen.getByText(/Migration required for updates/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /^update$/i })).toBeTruthy();
  });

  it("does not call a 1.6.0 install with one platform a legacy layout", async () => {
    renderRung(RUNG_1_6_0_UNPAIRED);
    expect(await screen.findByText(/hyprlayer 1\.6\.0/)).toBeTruthy();
    expect(screen.queryByText(/Legacy layout/)).toBeNull();
    expect(screen.queryByText(/Migration required/)).toBeNull();
  });

  it("offers the bundle catch-up once the CLI is current", async () => {
    renderRung(RUNG_BUNDLE_BEHIND);
    expect(await screen.findByRole("button", { name: /update to 1\.6\.2/i })).toBeTruthy();
    expect(screen.getByText(/1\.6\.2 available/)).toBeTruthy();
    expect(screen.queryByText(/Legacy layout/)).toBeNull();
  });

  it("settles on Latest with nothing left to do", async () => {
    renderRung(RUNG_CURRENT);
    expect(await screen.findByText(/Latest/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^update( to|$)/i })).toBeNull();
    expect(screen.queryByText(/available/)).toBeNull();
  });

  it("never offers the CLI upgrade twice on any rung", async () => {
    for (const rung of [RUNG_1_5_X, RUNG_1_6_0_UNPAIRED, RUNG_BUNDLE_BEHIND, RUNG_CURRENT]) {
      renderRung(rung);
      await screen.findByText(/Agent bundle/);
      expect(screen.queryAllByRole("button", { name: /^update$/i }).length).toBeLessThanOrEqual(1);
      cleanup();
    }
  });
});

describe("HyprlayerCard agent targets", () => {
  it("repairs in place instead of opening a terminal", async () => {
    b.repairAgentBundle.mockReturnValue(ok(repaired()));
    renderCard();
    fireEvent.click(await repairButton());
    await waitFor(() => expect(b.repairAgentBundle).toHaveBeenCalled());
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("calls a successful repair done, whatever source the CLI downloaded from", async () => {
    b.repairAgentBundle.mockReturnValue(ok(repaired({ source: "fallback" })));
    renderCard();
    fireEvent.click(await repairButton());
    await waitFor(() => expect(b.repairAgentBundle).toHaveBeenCalled());
    await waitFor(async () => expect((await repairButton()).className).toContain("text-ok"));
    expect(screen.queryByText(/unverified/i)).toBeNull();
    expect(screen.queryByText(/did not confirm/i)).toBeNull();
  });

  it("shows the CLI's output on a failed repair, and lets it be dismissed", async () => {
    b.repairAgentBundle.mockReturnValue(
      ok(repaired({ ok: false, exitCode: 1, stderr: "error: could not reach github.com" })),
    );
    renderCard();
    fireEvent.click(await repairButton());
    expect(await screen.findByText(/The repair failed/)).toBeTruthy();
    expect(screen.getByText(/could not reach github\.com/)).toBeTruthy();
    expect((await repairButton()).className).toContain("text-bad");

    fireEvent.click(screen.getByRole("button", { name: /dismiss error/i }));
    await waitFor(() => expect(screen.queryByText(/The repair failed/)).toBeNull());
    expect((await repairButton()).className).not.toContain("text-bad");
  });
});
