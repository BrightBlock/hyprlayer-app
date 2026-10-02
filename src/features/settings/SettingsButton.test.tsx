// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AppSettings } from "@/lib/bindings";
import { ESCAPE_COMMAND_ID, runCommand } from "@/lib/keybindings/registry";
import { useThemeStore } from "@/lib/theme";

const q = vi.hoisted(() => ({
  mutate: vi.fn(),
  settings: {} as AppSettings,
  connection: { state: "connected", host: null } as { state: string; host: null },
  invalidate: vi.fn(),
}));

vi.mock("@/lib/queries", () => ({
  useBootstrap: () => ({
    data: { settings: q.settings, config: { state: "ready", user: "jt" }, binaries: [] },
  }),
  useSetAppSettings: () => ({ mutate: q.mutate, isError: false, error: null, reset: vi.fn() }),
  useConnectionStatus: () => ({ data: q.connection, isSuccess: true }),
  useHosts: () => ({ data: { hosts: [{ destination: "nexus", label: "Nexus" }], active: null } }),
  useHostProbes: () => ({ nexus: { ok: false, code: "remote_unreachable", at: 1 } }),
  useAgentFiles: () => ({ data: [] }),
  useAgentSkills: () => ({ data: [] }),
  useArchivedWorkstreams: () => ({ data: [] }),
}));

vi.mock("@/lib/notifications", () => ({
  invalidateNotificationSettingsCache: q.invalidate,
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    to,
    search,
    ...rest
  }: {
    children?: React.ReactNode;
    to: string;
    search?: { section?: string };
    [k: string]: unknown;
  }) => (
    <a href={search?.section ? `${to}?section=${search.section}` : to} {...rest}>
      {children}
    </a>
  ),
}));

import { SettingsButton } from "./SettingsButton";
import { closeSettingsPopover, useSettingsPopover } from "./settingsPopoverStore";

beforeEach(() => {
  q.settings = { notifications: { enabled: true, sound: false }, quit: { confirm: true } };
  q.connection = { state: "connected", host: null };
  useThemeStore.setState({ mode: "system" });
});

afterEach(() => {
  act(() => closeSettingsPopover());
  cleanup();
  vi.clearAllMocks();
});

const gear = () => screen.getByRole("button", { name: "Settings" });
const open = () => fireEvent.click(gear());

describe("SettingsButton", () => {
  it("opens a labelled dialog and reflects it on the gear", () => {
    render(<SettingsButton />);
    expect(gear().getAttribute("aria-expanded")).toBe("false");
    open();
    expect(screen.getByRole("dialog", { name: "Settings" })).toBeTruthy();
    expect(gear().getAttribute("aria-expanded")).toBe("true");
  });

  it("closes on Escape and hands focus back to the gear", () => {
    render(<SettingsButton />);
    gear().focus();
    open();
    expect(document.activeElement).not.toBe(gear());
    act(() => {
      runCommand(ESCAPE_COMMAND_ID);
    });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(gear());
  });

  it("turns notifications off without touching the sound choice, then refreshes the cache", () => {
    render(<SettingsButton />);
    open();
    fireEvent.click(screen.getByRole("switch", { name: "Notify when a session needs me" }));
    const [next, options] = q.mutate.mock.calls[0];
    expect(next.notifications).toEqual({ enabled: false, sound: false });
    options.onSuccess();
    expect(q.invalidate).toHaveBeenCalled();
  });

  it("writes the quit confirmation to the same settings the full view reads", () => {
    render(<SettingsButton />);
    open();
    fireEvent.click(screen.getByRole("switch", { name: "Confirm before quitting" }));
    expect(q.mutate.mock.calls[0][0].quit).toEqual({ confirm: false });
  });

  it("writes the default harness", () => {
    render(<SettingsButton />);
    open();
    fireEvent.click(screen.getByRole("button", { name: "Default harness" }));
    fireEvent.click(screen.getByRole("option", { name: "Codex" }));
    expect(q.mutate.mock.calls[0][0].providers.defaultHarness).toBe("codex");
  });

  it("switches appearance through the segments, with Auto meaning the OS", () => {
    render(<SettingsButton />);
    open();
    expect(screen.getByRole("radio", { name: "Auto" }).getAttribute("aria-checked")).toBe("true");
    fireEvent.click(screen.getByRole("radio", { name: "Dark" }));
    expect(useThemeStore.getState().mode).toBe("dark");
  });

  it("selects no segment under a named theme", () => {
    useThemeStore.setState({ mode: "cobalt" });
    render(<SettingsButton />);
    open();
    for (const name of ["Light", "Dark", "Auto"]) {
      expect(screen.getByRole("radio", { name }).getAttribute("aria-checked")).toBe("false");
    }
  });

  it("links out to the Workshop, the Servers section, and General", () => {
    render(<SettingsButton />);
    open();
    expect(screen.getByRole("link", { name: /Workshop/ }).getAttribute("href")).toBe("/agents");
    expect(screen.getByRole("link", { name: /Servers/ }).getAttribute("href")).toBe(
      "/settings?section=servers",
    );
    expect(screen.getByRole("link", { name: /All settings/ }).getAttribute("href")).toBe("/settings");
    expect(screen.getByRole("link", { name: /Servers/ }).textContent).toContain("2 · 1 offline");
  });

  it("stays shut while the server connection is down, so it never stacks over recovery", () => {
    q.connection = { state: "lost", host: null };
    render(<SettingsButton />);
    open();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useSettingsPopover.getState().open).toBe(false);
  });
});
