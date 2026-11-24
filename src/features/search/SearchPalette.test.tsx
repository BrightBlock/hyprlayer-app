// @vitest-environment jsdom
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ESCAPE_COMMAND_ID, runCommand } from "@/lib/keybindings/registry";
import type { SearchResult } from "./search";
import { closeSearch, openSearch, useSearchStore } from "./searchStore";
import { SearchPalette } from "./SearchPalette";

const RESULTS: SearchResult[] = [
  { kind: "doc", key: "d1", title: "Plan A", score: 2, nav: { type: "doc", id: "d1" } },
  { kind: "doc", key: "d2", title: "Plan B", score: 1, nav: { type: "doc", id: "d2" } },
];
vi.mock("./useSearchSources", () => ({ useSearchSources: () => RESULTS }));
vi.mock("./selectAgentTab", () => ({ selectAgentTab: vi.fn() }));
vi.mock("@/features/session/workstreamTabs", () => ({ openDocPreviewTab: vi.fn() }));
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  useRouterState: ({ select }: { select: (s: unknown) => unknown }) =>
    select({ location: { pathname: "/", search: {} } }),
}));

Element.prototype.scrollIntoView = () => {};

afterEach(() => {
  cleanup();
  closeSearch();
});

describe("SearchPalette", () => {
  it("closes on Escape when a result row has focus", () => {
    render(<SearchPalette />);
    act(() => openSearch());
    screen.getByRole("option", { name: /Plan B/ }).focus();

    act(() => {
      runCommand(ESCAPE_COMMAND_ID);
    });

    expect(useSearchStore.getState().open).toBe(false);
  });

  it("returns focus to where it was when it closes", () => {
    render(
      <>
        <button>Opener</button>
        <SearchPalette />
      </>,
    );
    const opener = screen.getByRole("button", { name: "Opener" });
    opener.focus();
    act(() => openSearch());
    expect(document.activeElement).toBe(screen.getByPlaceholderText(/Search workstreams/));

    act(() => {
      runCommand(ESCAPE_COMMAND_ID);
    });

    expect(document.activeElement).toBe(opener);
  });
});
