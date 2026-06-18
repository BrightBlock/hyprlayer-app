// @vitest-environment jsdom
import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { isRegistered, runCommand, useCommand } from "./registry";

afterEach(cleanup);

function Harness({ id, handler }: { id: string; handler: (() => void) | null }) {
  useCommand(id, handler);
  return null;
}

describe("registry", () => {
  it("runCommand returns false when nothing is registered", () => {
    expect(runCommand("nope")).toBe(false);
  });

  it("registers a handler while mounted and runs it", () => {
    const fn = vi.fn();
    render(<Harness id="x.run" handler={fn} />);
    expect(isRegistered("x.run")).toBe(true);
    expect(runCommand("x.run")).toBe(true);
    expect(fn).toHaveBeenCalledOnce();
  });

  it("unregisters on unmount", () => {
    const { unmount } = render(<Harness id="x.unmount" handler={vi.fn()} />);
    expect(isRegistered("x.unmount")).toBe(true);
    unmount();
    expect(isRegistered("x.unmount")).toBe(false);
    expect(runCommand("x.unmount")).toBe(false);
  });

  it("a null handler leaves the command unregistered (context gate)", () => {
    render(<Harness id="x.gated" handler={null} />);
    expect(isRegistered("x.gated")).toBe(false);
  });

  it("runs the latest closure without re-registering churn", () => {
    const first = vi.fn();
    const second = vi.fn();
    const { rerender } = render(<Harness id="x.latest" handler={first} />);
    rerender(<Harness id="x.latest" handler={second} />);
    runCommand("x.latest");
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledOnce();
  });
});
