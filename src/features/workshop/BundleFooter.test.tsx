// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AgentBundleStatus } from "@/lib/bindings";

import { BundleFooter } from "./BundleFooter";

const q = vi.hoisted(() => ({ status: null as AgentBundleStatus | null }));
vi.mock("@/lib/queries", () => ({
  useAgentBundle: () => ({
    data: q.status,
    isLoading: false,
    isError: false,
    isFetching: false,
    refetch: vi.fn(),
  }),
  invalidateAgentBundle: vi.fn(),
}));

afterEach(cleanup);

function footer(status: AgentBundleStatus) {
  q.status = status;
  render(
    <QueryClientProvider client={new QueryClient()}>
      <BundleFooter />
    </QueryClientProvider>,
  );
}

describe("BundleFooter", () => {
  it("says the bundle is up to date on a CLI that installs a live one", () => {
    footer({
      manageable: true,
      installedSha: null,
      latestSha: null,
      installedVersion: "1.6.0",
      desiredVersion: "1.6.0",
      lastChecked: null,
      updateAvailable: false,
    });
    expect(screen.getByText(/^Up to date/)).toBeTruthy();
    expect(screen.queryByText("CLI out of date")).toBeNull();
  });

  it("blames the CLI, not the bundle, when the tree it tracks is frozen", () => {
    footer({
      manageable: true,
      installedSha: "9a2a33c0000000000000000000000000000000",
      latestSha: null,
      installedVersion: null,
      desiredVersion: null,
      lastChecked: null,
      updateAvailable: false,
    });
    expect(screen.getByText("CLI out of date")).toBeTruthy();
    expect(screen.queryByText(/^Up to date/)).toBeNull();
  });
});
