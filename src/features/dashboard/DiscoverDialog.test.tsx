// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DiscoverDialog } from "./DiscoverDialog";

const q = vi.hoisted(() => ({
  useBootstrap: vi.fn(),
  useRecentRepos: vi.fn(),
}));
vi.mock("@/lib/queries", () => q);

afterEach(cleanup);

type Repo = { nameWithOwner: string; description: string | null; url: string };

const repo = (nameWithOwner: string, extra: Record<string, unknown> = {}): Repo => ({
  nameWithOwner,
  description: null,
  url: `https://github.com/${nameWithOwner}.git`,
  ...extra,
});

const MULTI_ORG = [repo("jane/thoughts"), repo("acme/api"), repo("acme/web")];

function setup(recents: Record<string, unknown>, projects: Array<{ originUrl: string | null }> = []) {
  const refetch = vi.fn();
  const onPick = vi.fn();
  const onClose = vi.fn();
  q.useBootstrap.mockReturnValue({
    data: { config: { state: "ok" }, settings: { reposRoot: "/repos" }, projects },
  });
  const stub = (over: Record<string, unknown>) =>
    q.useRecentRepos.mockReturnValue({
      isLoading: false,
      isFetching: false,
      dataUpdatedAt: Date.now(),
      refetch,
      ...over,
    });

  stub(recents);
  // A fresh element each render: React bails out on an identical element ref.
  const dialog = () => <DiscoverDialog open onClose={onClose} onPick={onPick} />;
  const { rerender } = render(dialog());

  /** Swap what the query returns, as a refetch would, and re-render in place. */
  const rerenderWith = (over: Record<string, unknown>) => {
    stub(over);
    rerender(dialog());
  };
  return { refetch, onPick, onClose, rerenderWith };
}

const org = (name: RegExp) => screen.getByRole("button", { name });

describe("DiscoverDialog org rail", () => {
  it("scopes the list to the chosen organization", () => {
    setup({ data: MULTI_ORG });
    fireEvent.click(org(/^acme\s*2$/));
    expect(screen.queryByText("thoughts")).toBeNull();
    expect(screen.getByText("api")).toBeDefined();
    expect(screen.getByText("web")).toBeDefined();
  });

  it("counts repos per organization and in total", () => {
    setup({ data: MULTI_ORG });
    expect(org(/^All\s*3$/)).toBeDefined();
    expect(org(/^acme\s*2$/)).toBeDefined();
    expect(org(/^jane\s*1$/)).toBeDefined();
  });

  it("orders organizations by repo count, descending", () => {
    setup({ data: MULTI_ORG });
    const names = screen
      .getAllByRole("button", { pressed: false })
      .map((b) => b.textContent ?? "")
      .filter((t) => /^(acme|jane)/.test(t));
    expect(names[0]).toMatch(/^acme/);
    expect(names[1]).toMatch(/^jane/);
  });

  it("composes the rail scope with the text filter", () => {
    setup({ data: MULTI_ORG });
    fireEvent.click(org(/^acme\s*2$/));
    fireEvent.change(screen.getByLabelText("Filter repositories"), { target: { value: "web" } });
    expect(screen.queryByText("api")).toBeNull();
    expect(screen.getByText("web")).toBeDefined();
  });

  it("falls back to All when a refresh retires the selected organization", () => {
    const { rerenderWith } = setup({ data: MULTI_ORG });
    fireEvent.click(org(/^acme\s*2$/));
    expect(screen.queryByText("thoughts")).toBeNull();
    // The user left the acme org; the next fetch no longer carries its repos.
    rerenderWith({ data: [repo("jane/thoughts"), repo("bob/cli")] });
    expect(screen.getByText("thoughts")).toBeDefined();
    expect(screen.getByText("cli")).toBeDefined();
  });

  it("does not resurrect a retired organization when a later refresh returns it", () => {
    const { rerenderWith } = setup({ data: MULTI_ORG });
    fireEvent.click(org(/^acme\s*2$/));
    rerenderWith({ data: [repo("jane/thoughts")] });
    expect(screen.getByText("thoughts")).toBeDefined();
    // acme is back, but the user is browsing All now — don't yank the scope.
    rerenderWith({ data: MULTI_ORG });
    expect(screen.getByText("thoughts")).toBeDefined();
    expect(screen.getByText("api")).toBeDefined();
  });
});

describe("DiscoverDialog footer and row metadata", () => {
  it("reports the scoped repo count, not the filtered one", () => {
    setup({ data: MULTI_ORG });
    fireEvent.change(screen.getByLabelText("Filter repositories"), { target: { value: "web" } });
    expect(screen.getByText("3 repositories")).toBeDefined();
  });

  it("counts repos already cloned into projects, matching on the origin URL", () => {
    setup({ data: MULTI_ORG }, [
      { originUrl: "https://github.com/ACME/api.git" }, // case-insensitive
      { originUrl: "git@github.com:acme/web" }, // scp-style, no .git
      { originUrl: null }, // local-only project
    ]);
    expect(screen.getByText("2 already cloned")).toBeDefined();
  });

  it("omits the cloned count when nothing is cloned", () => {
    setup({ data: MULTI_ORG });
    expect(screen.queryByText(/already cloned/)).toBeNull();
  });

  it("shows a compact push age and marks private repos", () => {
    const twoDaysAgo = new Date(Date.now() - 2 * 86_400_000).toISOString();
    setup({ data: [repo("acme/api", { pushedAt: twoDaysAgo, private: true })] });
    expect(screen.getByText("2d")).toBeDefined();
    // The padlock is aria-hidden, so the row carries a text alternative.
    expect(screen.getByText("Private")).toBeDefined();
  });

  it("hands a picked repo to the Add-project wizard and closes", () => {
    // Discover never clones directly — the pick routes into the wizard so the
    // clone reviews the same location + thoughts steps as a pasted URL.
    const { onPick, onClose } = setup({ data: [repo("acme/api")] });
    fireEvent.click(screen.getByRole("button", { name: /acme\/api/ }));
    expect(onPick).toHaveBeenCalledWith(
      expect.objectContaining({ url: "https://github.com/acme/api.git" }),
    );
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("marks an already-cloned repo and refuses to pick it twice", () => {
    // Re-cloning would land on the existing directory and fail with repo_exists.
    const { onPick } = setup({ data: [repo("acme/api")] }, [
      { originUrl: "https://github.com/acme/api.git" },
    ]);
    expect(screen.getByText("Cloned")).toBeDefined();
    const row = screen.getByRole("button", { name: /acme\/api/ });
    expect(row).toHaveProperty("disabled", true);
    fireEvent.click(row);
    expect(onPick).not.toHaveBeenCalled();
  });

  it("ignores a non-GitHub origin that merely contains the github.com substring", () => {
    setup({ data: [repo("acme/api")] }, [{ originUrl: "https://notgithub.com/acme/api.git" }]);
    expect(screen.queryByText(/already cloned/)).toBeNull();
    expect(screen.queryByText("Cloned")).toBeNull();
  });
});

describe("DiscoverDialog refresh", () => {
  it("refetches from the rail footer", () => {
    const { refetch } = setup({ data: MULTI_ORG });
    fireEvent.click(screen.getByRole("button", { name: "Refresh repositories" }));
    expect(refetch).toHaveBeenCalledOnce();
  });

  it("stays available when gh returns no repos", () => {
    // The empty state is exactly when a refresh matters — after `gh auth login`.
    const { refetch } = setup({ data: [] });
    expect(screen.getByText(/No repositories found/)).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Refresh repositories" }));
    expect(refetch).toHaveBeenCalledOnce();
  });
});
