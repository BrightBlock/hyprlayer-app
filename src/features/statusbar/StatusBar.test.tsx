// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ConnectionStatus, Host, WorkstreamView } from "@/lib/bindings";
import { ESCAPE_COMMAND_ID, runCommand } from "@/lib/keybindings/registry";
import { useHostsUiStore } from "@/features/hosts/hostsUi";
import { useReconnectStore } from "@/features/hosts/reconnectState";
import { useServerAutoUpdate } from "@/features/hosts/serverAutoUpdate";
import { ServerUpdateConfirm } from "@/features/hosts/ServerUpdateConfirm";
import {
  controlsKey,
  useSessionControlsStore,
  type SessionControlActions,
  type SessionControlsSnapshot,
} from "@/features/session/sessionControls";
import { useWorkstreamTabsStore } from "@/features/session/workstreamTabs";
import { StatusBar } from "./StatusBar";

const b = vi.hoisted(() => ({ useConnectionStatus: vi.fn(), restartServer: vi.fn() }));
vi.mock("@/lib/queries", () => ({
  useConnectionStatus: b.useConnectionStatus,
}));
vi.mock("@/lib/bindings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/bindings")>();
  return { ...actual, commands: { ...actual.commands, restartServer: b.restartServer } };
});

const NOW = Date.UTC(2026, 8, 24, 12, 0, 0);
const NEXUS: Host = { destination: "nexus", label: "Nexus" };
const CONNECTED: ConnectionStatus = {
  state: "connected",
  host: null,
  serverVersion: 3,
  serverRelease: null,
  serverOutdated: false,
  transcriptSync: false,
  error: null,
};
const LOST: ConnectionStatus = {
  ...CONNECTED,
  state: "lost",
  host: NEXUS,
  error: { code: "server_disconnected", message: "The server connection ended.", remedy: null },
};
const FAILED: ConnectionStatus = {
  ...CONNECTED,
  state: "failed",
  host: NEXUS,
  serverVersion: null,
  error: { code: "remote_unreachable", message: "Could not reach nexus.", remedy: null },
};

function mockStatus(data: ConnectionStatus | undefined) {
  b.useConnectionStatus.mockReturnValue({ data, isSuccess: data !== undefined });
}

function makeView(
  over: Partial<{
    worktreeMode: WorkstreamView["item"]["worktreeMode"];
    exists: boolean;
    headBranch: string | null;
    additions: number;
    deletions: number;
  }> = {},
): WorkstreamView {
  return {
    item: {
      id: "w1",
      projectPath: "/project",
      title: "Status bar",
      slug: "status-bar",
      branch: "feat/status-bar",
      worktreePath: "/worktrees/status-bar",
      baseBranch: "main",
      worktreeMode: over.worktreeMode ?? "isolated",
      stage: "implement",
      status: "active",
      prNumber: null,
      createdAt: null,
      updatedAt: null,
      archivedAt: null,
    },
    git: {
      exists: over.exists ?? true,
      branch: over.headBranch !== undefined ? over.headBranch : "feat/status-bar",
      dirty: 0,
      additions: over.additions ?? 0,
      deletions: over.deletions ?? 0,
      untracked: 0,
      ahead: 0,
      behind: 0,
      aheadOfBase: 0,
      thoughts: "ok",
    },
    seed: null,
  };
}

function controls(patch: Partial<SessionControlsSnapshot> = {}): SessionControlsSnapshot {
  return {
    harness: "claude",
    interactive: true,
    model: {
      label: "Opus 5",
      hint: "1M",
      activeKey: "claude:opus",
      groups: [
        {
          key: "claude",
          label: "Claude Code",
          items: [
            { key: "claude:opus", label: "Opus 5" },
            { key: "claude:sonnet", label: "Sonnet 5" },
          ],
        },
      ],
    },
    effort: {
      label: "High",
      activeKey: "high",
      items: [
        { key: "medium", label: "Medium" },
        { key: "high", label: "High" },
      ],
    },
    permissionKey: "default",
    planning: false,
    context: { tokens: 24_000, window: 200_000, title: "Context 24k / 200k" },
    ...patch,
  };
}

function publish(snapshot: SessionControlsSnapshot): SessionControlActions {
  const actions: SessionControlActions = {
    pickModel: vi.fn(),
    pickEffort: vi.fn(),
    setPermissionMode: vi.fn(),
    openHarnessSettings: vi.fn(),
  };
  // No tab state for w1 means its default tab, shell 1, has focus.
  useSessionControlsStore.setState({
    byShell: { [controlsKey("w1", 1)]: { ...snapshot, actions, owner: {} } },
  });
  return actions;
}

function renderBar(workstream: WorkstreamView | null = makeView()) {
  return render(
    <StatusBar
      workstreamId={workstream?.item.id ?? null}
      workstream={workstream}
    />,
  );
}

const dot = (el: HTMLElement) => el.querySelector("span[aria-hidden]");

beforeEach(() => {
  mockStatus(CONNECTED);
  useHostsUiStore.setState({ open: false, menu: false, update: false });
  useReconnectStore.setState({ lostAt: null, attempting: false, halted: null });
  useServerAutoUpdate.setState({ updating: false });
  useSessionControlsStore.setState({ byShell: {} });
  useWorkstreamTabsStore.setState({ byWs: {} });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.clearAllMocks();
  Object.defineProperty(window, "innerWidth", { value: 1024, configurable: true });
});

describe("StatusBar sync status", () => {
  it("opens the server menu, not the hosts dialog, on click", () => {
    renderBar();
    fireEvent.click(screen.getByRole("button", { name: "Server: this machine" }));
    expect(useHostsUiStore.getState()).toMatchObject({ menu: true, open: false });
  });

  it("opens the server dialog, not the menu, while the connection is down", () => {
    mockStatus(LOST);
    renderBar();
    fireEvent.click(screen.getByRole("button", { name: "Connection to Nexus lost" }));
    expect(useHostsUiStore.getState()).toMatchObject({ open: true, menu: false });
  });

  it("reads 'connected' with a green dot when local and connected", () => {
    renderBar();
    const button = screen.getByRole("button", { name: "Server: this machine" });
    expect(button.title).toBe("Server: this machine");
    expect(button.textContent).toContain("connected");
    expect(dot(button)?.className).toContain("bg-ok");
  });

  it("names the host with a green dot when remote and connected", () => {
    mockStatus({ ...CONNECTED, host: NEXUS });
    renderBar();
    const button = screen.getByRole("button", { name: "Server: Nexus" });
    expect(button.textContent).toContain("connected · Nexus");
    expect(dot(button)?.className).toContain("bg-ok");
  });

  it("warns when the server is outdated, and offers its update", () => {
    mockStatus({ ...CONNECTED, host: NEXUS, serverRelease: "0.8.10", serverOutdated: true });
    renderBar();
    const button = screen.getByRole("button", { name: "Server update available on Nexus" });
    expect(dot(button)?.className).toContain("bg-warn");
    expect(screen.getByRole("button", { name: "Update server" }).title).toBe(
      "Update the server on Nexus",
    );
  });

  it("offers no update to a current server", () => {
    renderBar();
    expect(screen.queryByRole("button", { name: /^Update/ })).toBeNull();
  });

  it("asks before updating, naming what the update stops", async () => {
    mockStatus({ ...CONNECTED, serverRelease: "0.8.10", serverOutdated: true });
    b.restartServer.mockResolvedValue({ status: "ok", data: null });
    render(
      <>
        <StatusBar workstreamId="w1" workstream={makeView()} />
        <ServerUpdateConfirm />
      </>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Update server" }));
    expect(b.restartServer).not.toHaveBeenCalled();
    const confirm = screen.getByRole("dialog", { name: "Update the server?" });
    expect(confirm.textContent).toContain("Every agent and terminal running on this machine will stop.");

    await act(async () => {
      fireEvent.click(within(confirm).getByRole("button", { name: "Update server" }));
    });
    expect(b.restartServer).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog", { name: "Update the server?" })).toBeNull();
    expect(useServerAutoUpdate.getState().updating).toBe(true);
  });

  it("reads 'updating' through the planned drop, with no counter, retry or update offer", () => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
    useServerAutoUpdate.setState({ updating: true });
    mockStatus({ ...CONNECTED, serverRelease: "0.8.10", serverOutdated: true });
    const { rerender } = renderBar();
    const button = screen.getByRole("button", { name: "Updating the server on this machine" });
    expect(button.textContent).toBe("updating");
    expect(dot(button)?.className).toContain("bg-warn");
    expect(screen.queryByRole("button", { name: /^Update/ })).toBeNull();

    mockStatus({ ...LOST, host: null });
    useReconnectStore.setState({ lostAt: NOW - 9_000, attempting: true });
    rerender(<StatusBar workstreamId="w1" workstream={makeView()} />);
    expect(button.textContent).toBe("updating");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("names the host and shows a bad dot when never connected", () => {
    mockStatus(FAILED);
    renderBar();
    const button = screen.getByRole("button", { name: "Not connected to Nexus" });
    expect(button.textContent).toContain("offline");
    expect(dot(button)?.className).toContain("bg-bad");
  });

  it("names the host and shows a warn dot when lost", () => {
    mockStatus(LOST);
    renderBar();
    const button = screen.getByRole("button", { name: "Connection to Nexus lost" });
    expect(dot(button)?.className).toContain("bg-warn");
  });

  it("renders nothing until the status resolves", () => {
    mockStatus(undefined);
    renderBar();
    expect(screen.queryByRole("button", { name: /Server|connect/i })).toBeNull();
  });

  it("counts the time since the drop each second, and stops once connected", () => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
    mockStatus(LOST);
    useReconnectStore.setState({ lostAt: NOW - 9_000 });
    const { rerender } = renderBar();
    const button = screen.getByRole("button", { name: "Connection to Nexus lost" });
    expect(button.textContent).toContain("offline · 9s");

    act(() => vi.advanceTimersByTime(1_000));
    expect(button.textContent).toContain("offline · 10s");

    mockStatus({ ...CONNECTED, host: NEXUS });
    rerender(
      <StatusBar workstreamId="w1" workstream={makeView()} />,
    );
    expect(screen.getByRole("button", { name: "Server: Nexus" }).textContent).toContain("connected · Nexus");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("registers no timers while connected", () => {
    vi.useFakeTimers();
    renderBar();
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("StatusBar repo cluster", () => {
  it("shows the branch and the uncommitted diff, compacted and signed", () => {
    renderBar(makeView({ additions: 11_000, deletions: 3_400 }));
    expect(screen.getByText("feat/status-bar")).toBeTruthy();
    const diff = screen.getByTitle("Uncommitted changes vs HEAD");
    expect(diff.textContent).toBe("+11k−3.4k");
    expect(within(diff).getByText("+11k").className).toContain("text-diff-add");
    expect(within(diff).getByText("−3.4k").className).toContain("text-diff-del");
  });

  it("reads 'detached' on a detached HEAD", () => {
    renderBar(makeView({ headBranch: null }));
    expect(screen.getByText("detached")).toBeTruthy();
  });

  it("is hidden without a workstream", () => {
    renderBar(null);
    expect(screen.queryByTitle("Uncommitted changes vs HEAD")).toBeNull();
    expect(screen.queryByText("feat/status-bar")).toBeNull();
  });

  it("is hidden for a folder workstream and a missing worktree", () => {
    renderBar(makeView({ worktreeMode: "folder" }));
    expect(screen.queryByText("feat/status-bar")).toBeNull();
    cleanup();
    renderBar(makeView({ exists: false }));
    expect(screen.queryByText("feat/status-bar")).toBeNull();
  });
});

describe("StatusBar session controls", () => {
  it("is hidden when the focused tab publishes no controls", () => {
    renderBar();
    expect(screen.queryByRole("button", { name: /^Model:/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /^Permission mode:/ })).toBeNull();
  });

  it("shows the context meter, the mode, the model and its effort as separate controls", () => {
    publish(controls());
    renderBar();
    const meter = screen.getByRole("meter", { name: "Context 24k / 200k" });
    expect(meter.textContent).toContain("ctx");
    expect(meter.textContent).toContain("12%");
    expect(screen.getByRole("button", { name: "Permission mode: Ask each time" }).textContent).toBe(
      "Ask",
    );
    expect(screen.getByRole("button", { name: "Model: Opus 5" }).textContent).toBe("Opus 5");
    expect(screen.getByRole("button", { name: "Effort: High" }).textContent).toBe("High");
  });

  it("opens the context details on click, as the composer's meter did", () => {
    publish(controls());
    renderBar();
    const trigger = screen.getByRole("button", { name: /^Context 24k \/ 200k/ });
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(trigger);
    expect(trigger.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("24.0k / 200.0k")).toBeTruthy();
    expect(screen.getByText("Used").nextElementSibling?.textContent).toBe("12.0%");
    expect(screen.getByText("Free space").nextElementSibling?.textContent).toBe("88.0%");
  });

  it("closes the context details with Escape", () => {
    publish(controls());
    renderBar();
    fireEvent.click(screen.getByRole("button", { name: /^Context 24k \/ 200k/ }));
    act(() => {
      runCommand(ESCAPE_COMMAND_ID);
    });
    expect(screen.queryByText("Free space")).toBeNull();
  });

  it("hides the meter without a real context window", () => {
    publish(controls({ context: null }));
    renderBar();
    expect(screen.queryByRole("meter")).toBeNull();
  });

  it("warns near the limit without painting it as an error", () => {
    publish(controls({ context: { tokens: 190_000, window: 200_000, title: "Context 190k / 200k" } }));
    renderBar();
    const meter = screen.getByRole("meter");
    expect(meter.innerHTML).toContain("bg-warn");
    expect(meter.innerHTML).not.toContain("bg-bad");
  });

  it("opens a mode menu on the checked mode, moves by arrow key, and hands focus back", () => {
    const actions = publish(controls());
    renderBar();
    const pill = screen.getByRole("button", { name: "Permission mode: Ask each time" });
    // Keyboard path: jsdom's click does not focus the button the way a key press leaves it.
    pill.focus();
    fireEvent.click(pill);

    const menu = screen.getByRole("menu", { name: "Permission mode" });
    const checked = within(menu).getByRole("menuitemradio", { name: "Ask each time" });
    expect(checked.getAttribute("aria-checked")).toBe("true");
    expect(document.activeElement).toBe(checked);

    fireEvent.keyDown(checked, { key: "ArrowDown" });
    const next = within(menu).getByRole("menuitemradio", { name: "Accept edits" });
    expect(document.activeElement).toBe(next);

    fireEvent.click(next);
    expect(actions.setPermissionMode).toHaveBeenCalledWith("acceptEdits");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(pill);
  });

  it("marks Bypass with a bad dot", () => {
    publish(controls({ permissionKey: "bypassPermissions" }));
    renderBar();
    const pill = screen.getByRole("button", { name: "Permission mode: Bypass permissions" });
    expect(pill.textContent).toBe("Bypass");
    expect(dot(pill)?.className).toContain("bg-bad");
  });

  it("has no mode pill and no effort control on OpenCode", () => {
    publish(controls({ harness: "opencode", permissionKey: null, effort: null }));
    renderBar();
    expect(screen.queryByRole("button", { name: /^Permission mode:/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /^Effort:/ })).toBeNull();
    expect(screen.getByRole("button", { name: "Model: Opus 5" })).toBeTruthy();
  });

  it("disables the controls once the session has ended", () => {
    publish(controls({ interactive: false }));
    renderBar();
    expect(
      (screen.getByRole("button", { name: /^Permission mode:/ }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect((screen.getByRole("button", { name: /^Model:/ }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    expect((screen.getByRole("button", { name: /^Effort:/ }) as HTMLButtonElement).disabled).toBe(
      true,
    );
  });

  it("offers models and the harness settings from the model menu, and no effort", () => {
    const actions = publish(controls());
    renderBar();
    fireEvent.click(screen.getByRole("button", { name: "Model: Opus 5" }));
    const menu = screen.getByRole("menu", { name: "Model" });
    expect(document.activeElement).toBe(within(menu).getByRole("menuitemradio", { name: "Opus 5" }));
    expect(within(menu).queryByRole("group", { name: "Effort" })).toBeNull();

    fireEvent.click(within(menu).getByRole("menuitemradio", { name: "Sonnet 5" }));
    expect(actions.pickModel).toHaveBeenCalledWith("claude:sonnet");

    fireEvent.click(screen.getByRole("button", { name: "Model: Opus 5" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /Harnesses/ }));
    expect(actions.openHarnessSettings).toHaveBeenCalled();
  });

  it("picks the effort from its own menu, opened on the current level", () => {
    const actions = publish(controls());
    renderBar();
    fireEvent.click(screen.getByRole("button", { name: "Effort: High" }));
    const menu = screen.getByRole("menu", { name: "Effort" });
    const current = within(menu).getByRole("menuitemradio", { name: "High" });
    expect(current.getAttribute("aria-checked")).toBe("true");
    expect(document.activeElement).toBe(current);

    fireEvent.click(within(menu).getByRole("menuitemradio", { name: "Medium" }));
    expect(actions.pickEffort).toHaveBeenCalledWith("medium");
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("will not pick an effort level the model cannot run", () => {
    const actions = publish(
      controls({
        effort: {
          label: "High",
          activeKey: "high",
          items: [
            { key: "high", label: "High" },
            { key: "ultracode", label: "Ultracode", disabled: true, hint: "needs an xhigh-capable model" },
          ],
        },
      }),
    );
    renderBar();
    fireEvent.click(screen.getByRole("button", { name: "Effort: High" }));
    const blocked = within(screen.getByRole("menu", { name: "Effort" })).getByRole("menuitemradio", {
      name: /Ultracode/,
    });
    expect((blocked as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(blocked);
    expect(actions.pickEffort).not.toHaveBeenCalled();
  });
});

describe("StatusBar trailing items", () => {
  it("puts the tools slot after the session controls, before the right cluster", () => {
    render(
      <StatusBar
        workstreamId={null}
        workstream={null}
        tools={<button type="button">Terminal</button>}
      />,
    );
    const group = screen.getByRole("button", { name: "Terminal" }).parentElement;
    expect(group?.parentElement?.tagName).toBe("FOOTER");
    expect(group?.nextElementSibling?.className).toContain("border-l");
  });

  it("puts the sessions slot flush against the Server button, one divider after thoughts", () => {
    render(
      <StatusBar
        workstreamId="w1"
        workstream={makeView()}
        thoughts={<button type="button">Thoughts</button>}
        sessions={<button type="button">Agent sessions</button>}
      />,
    );
    const sessions = screen.getByRole("button", { name: "Agent sessions" });
    expect(sessions.previousElementSibling).toBeNull();
    expect(sessions.nextElementSibling).toBe(screen.getByRole("button", { name: "Server: this machine" }));
    const divider = sessions.parentElement!.previousElementSibling!;
    expect(divider.getAttribute("aria-hidden")).toBe("true");
    expect(divider.previousElementSibling).toBe(screen.getByRole("button", { name: "Thoughts" }));
  });


  it("leads with the sessions slot when there is no repo cluster", () => {
    render(
      <StatusBar
        workstreamId="w1"
        workstream={makeView({ worktreeMode: "folder" })}
        sessions={<button type="button">Agent sessions</button>}
      />,
    );
    const sessions = screen.getByRole("button", { name: "Agent sessions" });
    expect(sessions.previousElementSibling).toBeNull();
    expect(sessions.parentElement!.previousElementSibling).toBeNull();
  });
});

describe("StatusBar at compact width", () => {
  it("drops the diff and the sync text but keeps the context meter, the composer no longer shows one", () => {
    Object.defineProperty(window, "innerWidth", { value: 500, configurable: true });
    mockStatus(LOST);
    publish(controls());
    renderBar(makeView({ additions: 12, deletions: 3 }));
    expect(screen.queryByTitle("Uncommitted changes vs HEAD")).toBeNull();
    expect(screen.getByRole("meter", { name: "Context 24k / 200k" })).toBeTruthy();
    const sync = screen.getByRole("button", { name: "Connection to Nexus lost" });
    expect(sync.textContent).toBe("");
    expect(dot(sync)).not.toBeNull();
    expect(screen.getByText("feat/status-bar")).toBeTruthy();
  });
});
