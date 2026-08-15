// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DropdownSelect, Field, type DropdownOption } from "./ui";

afterEach(cleanup);

const OPTIONS: DropdownOption[] = [
  { value: "opus", label: "Opus 5", group: "Claude" },
  { value: "sonnet", label: "Sonnet 5", group: "Claude" },
  { value: "restricted", label: "Restricted", group: "Claude", disabled: true },
  { value: "gpt", label: "GPT-5.4", group: "Codex" },
];

function renderSelect(props: Partial<Parameters<typeof DropdownSelect>[0]> = {}) {
  const onChange = vi.fn();
  render(
    <DropdownSelect
      aria-label="Model"
      value="sonnet"
      options={OPTIONS}
      onChange={onChange}
      {...props}
    />,
  );
  return { onChange };
}

const trigger = () => screen.getByRole("button", { name: "Model" });
const openMenu = () => fireEvent.click(trigger());

describe("DropdownSelect", () => {
  it("wears the selected option's label, not its raw value", () => {
    renderSelect();
    expect(trigger().textContent).toBe("Sonnet 5");
  });

  it("falls back to the placeholder when the value matches nothing", () => {
    // A stored choice the catalog no longer offers must not leak its raw id
    // onto the button.
    renderSelect({ value: "retired-model", placeholder: "Pick a model" });
    expect(trigger().textContent).toBe("Pick a model");
  });

  it("renders options only while open, and reports the selection by value", () => {
    const { onChange } = renderSelect();
    expect(screen.queryByRole("option")).toBeNull();

    openMenu();
    fireEvent.click(screen.getByRole("option", { name: "Opus 5" }));

    expect(onChange).toHaveBeenCalledWith("opus");
    // Picking closes the menu.
    expect(screen.queryByRole("option")).toBeNull();
  });

  it("marks the current choice with aria-selected and keeps the ✓ out of the name", () => {
    renderSelect();
    openMenu();
    // Would read "Sonnet 5✓" if the glyph weren't aria-hidden.
    expect(screen.getByRole("option", { name: "Sonnet 5" }).getAttribute("aria-selected")).toBe(
      "true",
    );
    expect(screen.getByRole("option", { name: "Opus 5" }).getAttribute("aria-selected")).toBe(
      "false",
    );
  });

  it("heads each run of grouped options once", () => {
    renderSelect();
    openMenu();
    expect(screen.getAllByText("Claude")).toHaveLength(1);
    expect(screen.getAllByText("Codex")).toHaveLength(1);
  });

  it("refuses to select a disabled option", () => {
    const { onChange } = renderSelect();
    openMenu();
    fireEvent.click(screen.getByRole("option", { name: "Restricted" }));
    expect(onChange).not.toHaveBeenCalled();
  });

  it("opens on ArrowDown and walks only the enabled options", () => {
    renderSelect();
    fireEvent.keyDown(trigger(), { key: "ArrowDown" });

    const list = screen.getByRole("listbox");
    // Opening focuses the current choice…
    expect(document.activeElement).toBe(screen.getByRole("option", { name: "Sonnet 5" }));
    // …and ArrowDown skips "Restricted" to land on the next enabled row.
    fireEvent.keyDown(list, { key: "ArrowDown" });
    expect(document.activeElement).toBe(screen.getByRole("option", { name: "GPT-5.4" }));
    fireEvent.keyDown(list, { key: "Home" });
    expect(document.activeElement).toBe(screen.getByRole("option", { name: "Opus 5" }));
  });

  it("treats the trigger width as a floor, so long labels aren't truncated", () => {
    // Regression guard: the menu used to be pinned to `width: <trigger>`, so a
    // narrow settings-row trigger clipped its options to "Opus 4.…" — something
    // the native <select> it replaced never did, because the OS popup sized to
    // its content. jsdom has no layout, so assert the contract, not the pixels.
    renderSelect();
    openMenu();
    const style = screen.getByRole("listbox").style;
    expect(style.width).toBe("");
    expect(style.minWidth).not.toBe("");
    expect(style.maxWidth).toContain("100vw");
  });

  it("lets a caller's width class win over the default w-full", () => {
    // Tailwind sorts utilities into the stylesheet, so `w-full` beats a caller's
    // `w-56` regardless of class order — the component has to drop its own
    // default instead of appending both.
    renderSelect({ className: "w-56" });
    expect(trigger().className).toContain("w-56");
    expect(trigger().className).not.toMatch(/(^|\s)w-full(\s|$)/);
  });

  it("keeps w-full when the caller sets no width", () => {
    renderSelect({ className: "text-xs" });
    expect(trigger().className).toMatch(/(^|\s)w-full(\s|$)/);
  });

  it("stays open when the trigger sits inside a Field's label", () => {
    // Regression guard: WebKit forwards clicks on a <label> descendant to the
    // labeled control, so an inline (non-portaled) menu re-toggled the trigger
    // and could never be opened. See DropdownSelect's docs.
    render(
      <Field label="Model">
        <DropdownSelect aria-label="Model" value="sonnet" options={OPTIONS} onChange={vi.fn()} />
      </Field>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Model" }));
    expect(screen.getByRole("listbox")).toBeDefined();
  });
});
