// @vitest-environment jsdom
// Failure modes (one `it` each for 1-10; 11 lives in events.test.ts, 12 in agents.test.ts):
//  1. Signed out shows the thoughts handle's initials (old Avatar leaking) instead of the grey mark.
//  2. Signed in shows initials of something other than `author.name`.
//  3. A one-word name ("octocat") gives one letter instead of "OC".
//  4. An empty name renders an empty circle instead of the email's first letter (grey mark labelled Account when there is no email either).
//  5. `unsupported_command` shows a Sign-in button that can't work, or crashes, instead of the grey mark plus the remedy.
//  6. While disconnected, the popover opens over HostsDialog.
//  7. While signing in there is no Cancel, or Cancel doesn't call `account_cancel_sign_in`.
//  8. A cancelled sign-in (`account_sign_in_cancelled`) renders as an error.
//  9. A timeout/failure shows no message or remedy.
// 10. After sign-out the button still shows initials (stale query data).
// 11. `accountChanged` doesn't invalidate the ["account"] query.
// 12. The Collaborators tab appears once signed in.
import { QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AccountStatus } from "@/lib/bindings";
import { queryClient } from "@/lib/queryClient";

const b = vi.hoisted(() => ({
  accountStatus: vi.fn(),
  accountSignIn: vi.fn(),
  accountCancelSignIn: vi.fn(),
  accountSignOut: vi.fn(),
  connectionStatus: vi.fn(),
  getBootstrap: vi.fn(),
}));
vi.mock("@/lib/bindings", () => ({ commands: b }));
vi.mock("@/lib/log", () => ({ log: { warn: vi.fn(), info: vi.fn(), error: vi.fn() } }));

import { AccountButton } from "./AccountButton";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const fail = (code: string, message: string, remedy: string | null) =>
  Promise.resolve({ status: "error" as const, error: { code, message, remedy } });

const OUT: AccountStatus = { state: "signedOut" };
const signedIn = (name: string, email: string | null = "octo@hyprlayer.invalid"): AccountStatus => ({
  state: "signedIn",
  name,
  email,
});

function status(next: AccountStatus) {
  b.accountStatus.mockImplementation(() => ok(next));
}

function mount() {
  render(
    <QueryClientProvider client={queryClient}>
      <AccountButton />
    </QueryClientProvider>,
  );
}

const openPopover = async (name: string) => {
  fireEvent.click(await screen.findByRole("button", { name }));
  return screen.findByRole("dialog", { name: "Account" });
};

beforeEach(() => {
  b.connectionStatus.mockImplementation(() => ok({ state: "connected", host: null }));
  // The thoughts handle must never reach the button.
  b.getBootstrap.mockImplementation(() =>
    ok({ config: { state: "ready", path: "/c", user: "jon tsiros", backendKind: "git", configured: true } }),
  );
  b.accountCancelSignIn.mockImplementation(() => Promise.resolve());
  status(OUT);
});

afterEach(() => {
  cleanup();
  queryClient.clear();
  vi.clearAllMocks();
});

describe("AccountButton", () => {
  it("shows the grey mark, not the handle's initials, when signed out", async () => {
    mount();
    const button = await screen.findByRole("button", { name: "Sign in" });
    expect(button.getAttribute("title")).toBe("Sign in with GitHub");
    expect(button.textContent).toBe("");
    expect(screen.queryByText("JT")).toBeNull();
  });

  it("shows the initials of author.name when signed in", async () => {
    status(signedIn("Octo Cat"));
    mount();
    const button = await screen.findByRole("button", { name: "Account: Octo Cat" });
    expect(button.textContent).toBe("OC");
  });

  it("gives two letters for a one-word name", async () => {
    status(signedIn("octocat"));
    mount();
    const button = await screen.findByRole("button", { name: "Account: octocat" });
    expect(button.textContent).toBe("OC");
  });

  it("falls back to the email's first letter when the name yields no initials", async () => {
    status(signedIn("", "octo@hyprlayer.invalid"));
    mount();
    const button = await screen.findByRole("button", { name: "Account: octo@hyprlayer.invalid" });
    expect(button.textContent).toBe("O");
  });

  it("shows the grey mark labelled Account when neither name nor email gives a letter", async () => {
    status(signedIn("", null));
    mount();
    const button = await screen.findByRole("button", { name: "Account" });
    expect(button.textContent).toBe("");
  });

  it("shows the grey mark and the remedy, with no Sign in row, on an old server", async () => {
    b.accountStatus.mockImplementation(() =>
      fail("unsupported_command", "The server doesn't support this yet.", "Update the Hyprlayer server on this host."),
    );
    mount();
    const dialog = await openPopover("Account");
    expect(dialog.textContent).toContain("The server doesn't support this yet.");
    expect(dialog.textContent).toContain("Update the Hyprlayer server on this host.");
    expect(screen.queryByRole("button", { name: /Sign in with GitHub/ })).toBeNull();
  });

  it("stays closed while disconnected", async () => {
    b.connectionStatus.mockImplementation(() => ok({ state: "disconnected", host: null }));
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Sign in" }));
    await act(async () => {});
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("offers Cancel while signing in, and Cancel calls account_cancel_sign_in", async () => {
    b.accountSignIn.mockImplementation(() => new Promise(() => {}));
    mount();
    await openPopover("Sign in");
    fireEvent.click(await screen.findByRole("button", { name: /Sign in with GitHub/ }));
    expect(await screen.findByText("Waiting for GitHub…")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(b.accountCancelSignIn).toHaveBeenCalledTimes(1));
  });

  it("does not render a cancelled sign-in as an error", async () => {
    let settle!: () => void;
    b.accountSignIn.mockImplementation(
      () =>
        new Promise((resolve) => {
          settle = () =>
            resolve({
              status: "error",
              error: { code: "account_sign_in_cancelled", message: "Sign-in was cancelled.", remedy: null },
            });
        }),
    );
    mount();
    await openPopover("Sign in");
    fireEvent.click(await screen.findByRole("button", { name: /Sign in with GitHub/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
    await act(async () => settle());
    expect(await screen.findByRole("button", { name: /Sign in with GitHub/ })).toBeTruthy();
    expect(screen.queryByText("Sign-in was cancelled.")).toBeNull();
  });

  it("shows the message and remedy when sign-in fails", async () => {
    b.accountSignIn.mockImplementation(() =>
      fail("account_sign_in_timed_out", "GitHub sign-in timed out.", "Sign in again."),
    );
    mount();
    await openPopover("Sign in");
    fireEvent.click(await screen.findByRole("button", { name: /Sign in with GitHub/ }));
    expect(await screen.findByText("GitHub sign-in timed out.")).toBeTruthy();
    expect(screen.getByText("Sign in again.")).toBeTruthy();
  });

  it("returns to the grey mark right after sign-out", async () => {
    status(signedIn("Octo Cat"));
    b.accountSignOut.mockImplementation(() => ok(OUT));
    mount();
    await openPopover("Account: Octo Cat");
    fireEvent.click(await screen.findByRole("button", { name: "Sign out" }));
    expect(await screen.findByRole("button", { name: "Sign in" })).toBeTruthy();
    expect(screen.queryByText("OC")).toBeNull();
  });
});
