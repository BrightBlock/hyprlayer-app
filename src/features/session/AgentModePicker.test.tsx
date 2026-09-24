// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AgentModePicker } from "./AgentModePicker";

afterEach(cleanup);

describe("AgentModePicker", () => {
  it("is an icon in the mode's colour, with no visible text", () => {
    render(<AgentModePicker mode="research" started={false} onChange={vi.fn()} />);
    const trigger = screen.getByRole("button", { name: "Agent mode: Research" });
    expect(trigger.textContent).toBe("");
    expect(trigger.className).toContain("text-research");
    expect(trigger.querySelector("svg")).not.toBeNull();
  });

  it("opens the modes with the current one checked, each marked by its coloured icon", () => {
    render(<AgentModePicker mode="plan" started={false} onChange={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Agent mode: Plan" }));
    const menu = screen.getByRole("menu", { name: "Agent mode" });
    const items = within(menu).getAllByRole("menuitemradio");
    expect(items.map((i) => i.textContent?.replace("✓", "").trim())).toEqual([
      "General",
      "Research",
      "Plan",
      "Implement",
      "Validate",
    ]);
    const plan = within(menu).getByRole("menuitemradio", { name: /Plan/ });
    expect(plan.getAttribute("aria-checked")).toBe("true");
    expect(plan.querySelector(".text-plan svg")).not.toBeNull();
  });

  it("reports the picked mode and closes", () => {
    const onChange = vi.fn();
    render(<AgentModePicker mode="general" started={false} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: "Agent mode: General" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Implement/ }));
    expect(onChange).toHaveBeenCalledWith("implement");
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("is fixed once the session starts: the coloured icon stays, the menu does not open", () => {
    render(<AgentModePicker mode="implement" started onChange={vi.fn()} />);
    expect(screen.queryByRole("button")).toBeNull();
    const icon = screen.getByRole("img", { name: "Agent mode: Implement" });
    expect(icon.className).toContain("text-implement");
    fireEvent.click(icon);
    expect(screen.queryByRole("menu")).toBeNull();
  });
});
