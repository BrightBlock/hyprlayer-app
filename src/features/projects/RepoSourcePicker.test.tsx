// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { RepoSourcePicker } from "./RepoSourcePicker";

const q = vi.hoisted(() => ({ useRecentRepos: vi.fn() }));
vi.mock("@/lib/queries", () => q);
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

afterEach(cleanup);

const REPOS = [{ nameWithOwner: "jane/thoughts", description: null, url: "https://x/y.git" }];

function setup(overrides: Record<string, unknown> = {}) {
  const refetch = vi.fn();
  q.useRecentRepos.mockReturnValue({ data: REPOS, isLoading: false, isFetching: false, refetch, ...overrides });
  render(
    <RepoSourcePicker
      value={{ mode: "clone", url: "", location: "" }}
      onChange={vi.fn()}
      modes={["clone"]}
    />,
  );
  return { refetch };
}

describe("RepoSourcePicker recent repos", () => {
  it("refetches the repo list when the refresh button is clicked", () => {
    const { refetch } = setup();
    fireEvent.click(screen.getByRole("button", { name: "Refresh recent repos" }));
    expect(refetch).toHaveBeenCalledOnce();
  });

  it("disables refresh while a refetch is in flight", () => {
    setup({ isFetching: true });
    expect(screen.getByRole("button", { name: "Refresh recent repos" })).toHaveProperty(
      "disabled",
      true,
    );
  });

  it("keeps refresh reachable when gh returns no repos", () => {
    // The empty state is exactly when a refresh matters — after `gh auth login`.
    const { refetch } = setup({ data: [] });
    expect(screen.getByText(/None found/)).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Refresh recent repos" }));
    expect(refetch).toHaveBeenCalledOnce();
  });
});
