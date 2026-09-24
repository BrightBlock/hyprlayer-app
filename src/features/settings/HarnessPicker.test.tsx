// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { HarnessPicker } from "./HarnessPicker";

afterEach(cleanup);

describe("HarnessPicker radio", () => {
  function renderRadio(value = "claude") {
    const onChange = vi.fn();
    render(<HarnessPicker variant="radio" value={value} onChange={onChange} />);
    return { onChange, radio: (name: string) => screen.getByRole("radio", { name }) };
  }

  it("is one radiogroup of the four harnesses, with the default as its only tab stop", () => {
    const { radio } = renderRadio("codex");
    expect(screen.getByRole("radiogroup", { name: "Default harness" })).toBeTruthy();
    expect(screen.getAllByRole("radio")).toHaveLength(4);
    expect(radio("Codex").getAttribute("aria-checked")).toBe("true");
    expect(radio("Codex").tabIndex).toBe(0);
    expect(radio("Claude Code").tabIndex).toBe(-1);
  });

  it("selects the next harness with an arrow key and wraps", () => {
    const { onChange, radio } = renderRadio("claude");
    radio("Claude Code").focus();
    fireEvent.keyDown(radio("Claude Code"), { key: "ArrowRight" });
    expect(onChange).toHaveBeenLastCalledWith("codex");
    expect(document.activeElement).toBe(radio("Codex"));
    fireEvent.keyDown(radio("Codex"), { key: "ArrowLeft" });
    fireEvent.keyDown(radio("Claude Code"), { key: "ArrowLeft" });
    expect(onChange).toHaveBeenLastCalledWith("ollama");
  });

  it("selects with a click", () => {
    const { onChange, radio } = renderRadio("claude");
    fireEvent.click(radio("OpenCode"));
    expect(onChange).toHaveBeenCalledWith("opencode");
  });
});

describe("HarnessPicker dropdown", () => {
  it("shows the current harness and reports a new pick", () => {
    const onChange = vi.fn();
    render(<HarnessPicker variant="dropdown" value="claude" onChange={onChange} />);
    const trigger = screen.getByRole("button", { name: "Default harness" });
    expect(trigger.textContent).toContain("Claude Code");
    fireEvent.click(trigger);
    fireEvent.click(screen.getByRole("option", { name: "Codex" }));
    expect(onChange).toHaveBeenCalledWith("codex");
  });
});
