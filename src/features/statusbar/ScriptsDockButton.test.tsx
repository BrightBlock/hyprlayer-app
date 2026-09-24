// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ScriptsDockButton } from "./ScriptsDockButton";

afterEach(cleanup);

function renderButton(props: Partial<Parameters<typeof ScriptsDockButton>[0]> = {}) {
  const onToggle = vi.fn();
  render(<ScriptsDockButton open={false} running={false} onToggle={onToggle} {...props} />);
  return { onToggle, button: screen.getByRole("button", { name: "Terminal and scripts dock" }) };
}

describe("ScriptsDockButton", () => {
  it("toggles the dock and reports whether it is open", () => {
    const { onToggle, button } = renderButton({ open: true });
    expect(button.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(button);
    expect(onToggle).toHaveBeenCalledTimes(1);
  });

  it("says a Run is active and marks it with a dot", () => {
    const { button } = renderButton({ running: true });
    expect(button.title).toBe("Scripts · a Run is active");
    expect(button.querySelector("span[aria-hidden]")).not.toBeNull();
  });

  it("shows no dot while nothing runs", () => {
    const { button } = renderButton();
    expect(button.title).toBe("Terminal & scripts dock");
    expect(button.querySelector("span[aria-hidden]")).toBeNull();
  });

});
