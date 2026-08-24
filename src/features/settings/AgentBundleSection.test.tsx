// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AgentBundleInstall, AiStatus, AiVersions } from "@/lib/bindings";

import { AgentBundleSection } from "./AgentBundleSection";

const b = vi.hoisted(() => ({
  installAgentBundleVersion: vi.fn(),
  agentBundleVersions: vi.fn(),
  agentBundleStatus: vi.fn(),
}));
vi.mock("@/lib/bindings", () => ({
  commands: {
    installAgentBundleVersion: b.installAgentBundleVersion,
    agentBundleVersions: b.agentBundleVersions,
    agentBundleStatus: b.agentBundleStatus,
  },
}));

// The section's "Manage Agents" link; jsdom has no router to resolve it.
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, ...rest }: { children?: unknown; [k: string]: unknown }) => (
    <a {...rest}>{children as never}</a>
  ),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

/** Freshness probe behind the row's "checked …" note and — via `manageable` —
 *  the gate on every interactive control here. */
const FRESHNESS = {
  manageable: true,
  installedSha: "9a2a33c6fc0648e8cd7bac278365c8a9dcca220b",
  latestSha: "9a2a33c6fc0648e8cd7bac278365c8a9dcca220b",
  lastChecked: null,
  updateAvailable: false,
};

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });

const STATUS: AiStatus = {
  agentTool: "Claude Code",
  agentsInstalledSha: "9a2a33c",
  assetsVersion: "1.6.0",
  binaryVersion: "1.6.0",
  installed: true,
  lastAgentCheck: null,
  location: "~/.claude/",
  pinnedVersion: "1.6.0",
};

const VERSIONS: AiVersions = {
  assetsVersion: "1.6.0",
  binaryVersion: "1.6.0",
  harness: "claude",
  pinnedVersion: "1.6.0",
  versions: [
    { version: "1.6.0", publishedAt: "2026-08-24T04:13:17Z", current: true, pinned: true },
    { version: "1.6.0-rc.2", publishedAt: "2026-08-24T03:17:01Z", current: false, pinned: false },
  ],
};

const install = (over: Partial<AgentBundleInstall>): AgentBundleInstall => ({
  ok: true,
  exitCode: 0,
  source: "releaseAsset",
  stdout: "  Downloaded 54 files from the v1.6.0 release asset\n",
  stderr: "",
  backupPath: "/home/u/.claude/hyprlayer-backup",
  ...over,
});

function renderSection(status: AiStatus | undefined = STATUS, freshness: unknown = FRESHNESS) {
  b.agentBundleStatus.mockReturnValue(ok(freshness));
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <AgentBundleSection status={status} />
    </QueryClientProvider>,
  );
}

/** Open the listbox. The trigger only mounts once the freshness probe has
 *  answered `manageable`, so finding it already waits for that; the version
 *  list itself is fetched lazily by this very open. */
async function openMenu() {
  const trigger = (await screen.findByRole("button", {
    name: /agent bundle version/i,
  })) as HTMLButtonElement;
  fireEvent.click(trigger);
  return trigger;
}

/** Open the listbox and pick an option by its visible label. */
async function choose(match: RegExp) {
  await openMenu();
  fireEvent.click(await screen.findByRole("option", { name: match }));
}

describe("AgentBundleSection", () => {
  // One row, not two: the version stands where the install SHA used to, and is
  // itself the control.
  it("names the installed version in the bundle row, as the picker trigger", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    renderSection();
    const trigger = await screen.findByRole("button", { name: /agent bundle version/i });
    expect(trigger.textContent).toContain("1.6.0");
    expect(screen.getByText(/Agent bundle/)).toBeTruthy();
  });

  // The list spawns a CLI process that can reach GitHub; a Settings visit that
  // never opens the menu must never pay for it.
  it("fetches the version list only when the picker is opened", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    renderSection();
    await screen.findByRole("button", { name: /agent bundle version/i });
    expect(b.agentBundleVersions).not.toHaveBeenCalled();
    await openMenu();
    await waitFor(() => expect(b.agentBundleVersions).toHaveBeenCalled());
  });

  // `ai versions` follows the CLI's configured harness, and the install
  // command refuses anything but Claude Code — so on another tool the picker
  // would be a dead-end control. The row degrades to the read-only SHA.
  it("shows a read-only SHA instead of a picker when the tool is not manageable", async () => {
    renderSection({ ...STATUS, agentTool: "OpenCode" }, { ...FRESHNESS, manageable: false });
    expect(await screen.findByText(/9a2a33c/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /agent bundle version/i })).toBeNull();
    expect(screen.queryByText(/Update the hyprlayer CLI/)).toBeNull();
    expect(b.agentBundleVersions).not.toHaveBeenCalled();
  });

  // A 1.6.0 CLI over a bundle installed before versioned assets existed:
  // `assetsVersion` is null but the picker is live, and the SHA is still the
  // bundle's only identity — not the literal "unknown".
  it("falls back to the install SHA when the bundle predates versioned assets", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    renderSection({ ...STATUS, assetsVersion: null, pinnedVersion: null });
    const trigger = await screen.findByRole("button", { name: /agent bundle version/i });
    await waitFor(() => expect(trigger.textContent).toContain("9a2a33c"));
    expect(trigger.textContent).not.toContain("unknown");
  });

  // Picking a version is what pins it, so saying "pinned" narrates the control
  // the user just used. Being pinned is surfaced by Clear pin, which is
  // actionable; only a pin that disagrees with the binary gets words.
  it("does not narrate pin state on the row", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    renderSection();
    await screen.findByRole("button", { name: /agent bundle version/i });
    expect(screen.queryByText(/not pinned/)).toBeNull();
    expect(screen.queryByText(/·\s*pinned/)).toBeNull();
    // ...but the affordance that reveals the pin is still there.
    expect(screen.getByRole("button", { name: /clear pin/i })).toBeTruthy();
  });

  it("shows no separate SHA row alongside the version", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    renderSection();
    await screen.findByRole("button", { name: /agent bundle version/i });
    expect(screen.queryByText(/9a2a33c/)).toBeNull();
  });

  // The state users get stuck in: the pin keeps reinstalling a bundle the
  // current binary did not ship, on every upgrade, silently.
  it("says so when the pin no longer matches the binary", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    renderSection({ ...STATUS, pinnedVersion: "1.6.0-rc.2", binaryVersion: "1.6.0" });
    expect(await screen.findByText(/this CLI ships 1\.6\.0/)).toBeTruthy();
  });

  it("offers one option per listed version and nothing else", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    renderSection();
    await openMenu();
    // Wait past the lazy fetch's disabled "Loading versions…" row.
    await screen.findByRole("option", { name: /1\.6\.0-rc\.2/ });
    const options = screen.getAllByRole("option");
    expect(options.map((o) => o.textContent)).toEqual([
      expect.stringContaining("1.6.0"),
      expect.stringContaining("1.6.0-rc.2"),
    ]);
  });

  // The arm/disarm rule: the icon means "this is not what you have". On an
  // unpinned machine the pin is null, so a pin-only comparison could never
  // disarm once anything was picked — returning to the installed version has
  // to read as "no change".
  it("disarms the install affordance when the selection returns to the current version", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    renderSection({ ...STATUS, pinnedVersion: null });
    await choose(/1\.6\.0-rc\.2/);
    expect(await screen.findByRole("button", { name: /install 1\.6\.0-rc\.2/i })).toBeTruthy();
    await choose(/1\.6\.0 ·/);
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: /^install/i })).toBeNull(),
    );
    expect(b.installAgentBundleVersion).not.toHaveBeenCalled();
  });

  it("installs the chosen version only after an explicit confirm", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    b.installAgentBundleVersion.mockReturnValue(ok(install({})));
    renderSection();
    await choose(/1\.6\.0-rc\.2/);
    // Choosing alone must not rewrite ~/.claude.
    expect(b.installAgentBundleVersion).not.toHaveBeenCalled();
    fireEvent.click(await screen.findByRole("button", { name: /install 1\.6\.0-rc\.2/i }));
    await waitFor(() => expect(b.installAgentBundleVersion).toHaveBeenCalledWith("1.6.0-rc.2"));
  });

  it("clears the pin with a null version", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    b.installAgentBundleVersion.mockReturnValue(ok(install({})));
    renderSection();
    fireEvent.click(await screen.findByRole("button", { name: /clear pin/i }));
    await waitFor(() => expect(b.installAgentBundleVersion).toHaveBeenCalledWith(null));
  });

  it("offers no Clear pin when nothing is pinned", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    renderSection({ ...STATUS, pinnedVersion: null });
    await screen.findByRole("button", { name: /agent bundle version/i });
    expect(screen.queryByRole("button", { name: /clear pin/i })).toBeNull();
    expect(screen.queryByText(/not pinned/)).toBeNull();
  });

  /**
   * The whole point of reading stdout. This run exits 0 and the config records
   * the requested version, so every other signal available says "success"
   * while `~/.claude` holds master's tree instead.
   */
  it("reports a silent fallback install as a problem, not a success", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    b.installAgentBundleVersion.mockReturnValue(
      ok(
        install({
          source: "fallback",
          stdout:
            "  Fetching the v1.6.0-rc.2 release asset failed (…); trying the next source.\n  Downloaded 60 files from the master repo archive\n",
        }),
      ),
    );
    renderSection();
    await choose(/1\.6\.0-rc\.2/);
    fireEvent.click(await screen.findByRole("button", { name: /install 1\.6\.0-rc\.2/i }));
    expect(await screen.findByText(/no longer matches the version shown/i)).toBeTruthy();
    expect(screen.getByText(/hyprlayer-backup/)).toBeTruthy();
    expect(screen.queryByText(/Bundle reinstalled from the published release/)).toBeNull();
  });

  // A fallback on --unpin is the CLI legitimately installing the repository
  // tree because its own version has no published asset (a source-built or
  // nightly binary) — there is no "version shown" left to mismatch, so the
  // corruption warning would be a false alarm.
  it("reports an unpin that fell back as informational, not corruption", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    b.installAgentBundleVersion.mockReturnValue(
      ok(
        install({
          source: "fallback",
          stdout:
            "  Fetching the v1.7.0-dev release asset failed (…); trying the next source.\n  Downloaded 60 files from the master repo archive\n",
        }),
      ),
    );
    renderSection();
    fireEvent.click(await screen.findByRole("button", { name: /clear pin/i }));
    expect(await screen.findByText(/The pin was cleared/)).toBeTruthy();
    expect(screen.queryByText(/no longer matches the version shown/i)).toBeNull();
  });

  // The tripwire's decay mode: a reworded CLI success line lands on `unknown`,
  // and for a picked version "could not confirm" must not read calmer than
  // "confirmed bad".
  it("treats an unconfirmed source as a caution for a picked version", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    b.installAgentBundleVersion.mockReturnValue(
      ok(install({ source: "unknown", stdout: "Installed 54 files.\n" })),
    );
    renderSection();
    await choose(/1\.6\.0-rc\.2/);
    fireEvent.click(await screen.findByRole("button", { name: /install 1\.6\.0-rc\.2/i }));
    expect(await screen.findByText(/treat the installed files as unverified/i)).toBeTruthy();
  });

  // `ai reinstall` has no --json, so its own words are all the user gets.
  it("shows the CLI's output when the reinstall exits non-zero", async () => {
    b.agentBundleVersions.mockReturnValue(ok(VERSIONS));
    b.installAgentBundleVersion.mockReturnValue(
      ok(install({ ok: false, exitCode: 1, stderr: "Error: could not verify the bundle digest" })),
    );
    renderSection();
    await choose(/1\.6\.0-rc\.2/);
    fireEvent.click(await screen.findByRole("button", { name: /install 1\.6\.0-rc\.2/i }));
    expect(await screen.findByText(/exit 1/)).toBeTruthy();
    expect(screen.getByText(/could not verify the bundle digest/)).toBeTruthy();
    // The CLI can die mid-sync, so the failure copy must not claim the disk is
    // untouched — it names the kept pre-install copy instead...
    expect(screen.queryByText(/left as it was/i)).toBeNull();
    expect(screen.getByText(/hyprlayer-backup/)).toBeTruthy();
    // ...and the selection survives, so the Install button stands for a retry.
    expect(screen.getByRole("button", { name: /install 1\.6\.0-rc\.2/i })).toBeTruthy();
  });

  // Offline with a cold CLI cache. The row above is read from `ai status`,
  // which is local, so the list failing must not take the whole card down.
  it("degrades to a readable message when the version list cannot be fetched", async () => {
    b.agentBundleVersions.mockReturnValue(
      Promise.resolve({
        status: "error" as const,
        error: {
          code: "exec",
          message: "Unable to fetch the release list from GitHub: network error",
          remedy: null,
        },
      }),
    );
    renderSection();
    const trigger = await openMenu();
    expect(await screen.findByText(/Could not reach GitHub for the version list/i)).toBeTruthy();
    // Still names the installed version: it comes from `ai status`, not the list.
    expect(trigger.textContent).toContain("1.6.0");
  });

  it("says so when no release publishes a bundle", async () => {
    b.agentBundleVersions.mockReturnValue(ok({ ...VERSIONS, versions: [] }));
    renderSection();
    await openMenu();
    expect(await screen.findByText(/No recent release publishes an agent bundle/i)).toBeTruthy();
  });

  // A pre-1.6.0 binary has no `ai versions` to call at all.
  it("explains the CLI is too old instead of calling a command it lacks", async () => {
    renderSection({
      agentTool: "Claude Code",
      installed: true,
      location: "~/.claude/",
      agentsInstalledSha: "abc",
      lastAgentCheck: null,
    });
    expect(await screen.findByText(/Update the hyprlayer CLI to 1\.6\.0 or newer/i)).toBeTruthy();
    expect(b.agentBundleVersions).not.toHaveBeenCalled();
  });
});
