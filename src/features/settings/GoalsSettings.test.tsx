// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AppSettings } from "@/lib/bindings";
import { GoalsSettings } from "./GoalsSettings";

const m = vi.hoisted(() => ({ mutate: vi.fn() }));
vi.mock("@/lib/queries", () => ({
  useSetAppSettings: () => ({ mutate: m.mutate, isError: false, error: null, reset: vi.fn() }),
  useOpencodeCatalog: () => ({ data: undefined }),
  useClaudeProbe: () => ({ data: undefined }),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("GoalsSettings", () => {
  it("saves 1 active minute over the default 60", () => {
    render(<GoalsSettings settings={{} as AppSettings} />);
    const input = screen.getByLabelText("Active minutes") as HTMLInputElement;
    expect(input.value).toBe("60");

    fireEvent.change(input, { target: { value: "1" } });
    fireEvent.blur(input);

    expect(m.mutate).toHaveBeenCalledTimes(1);
    expect(m.mutate.mock.calls[0][0].goals.maxActiveSecs).toBe(60);
  });
});
