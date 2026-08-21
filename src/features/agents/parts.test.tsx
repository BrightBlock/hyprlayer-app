// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { FolderCombo } from "./parts";

afterEach(cleanup);

const FOLDERS = ["Discover", "Design", "Review", "Any stage"];

function combo(props: Partial<Parameters<typeof FolderCombo>[0]> = {}) {
  const onChange = vi.fn();
  render(
    <FolderCombo
      id="test"
      value=""
      onChange={onChange}
      folders={FOLDERS}
      placeholder="No folder"
      aria-label="Folder"
      {...props}
    />,
  );
  return { input: screen.getByRole("combobox", { name: "Folder" }), onChange };
}

describe("FolderCombo", () => {
  it("draws its own listbox, not a native datalist", () => {
    // The datalist this replaced took none of the app's styling — the panel
    // has to be the app's own markup for any of it to apply.
    const { input } = combo();
    fireEvent.click(input);
    expect(document.querySelector("datalist")).toBeNull();
    expect(screen.getByRole("listbox")).toBeTruthy();
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(FOLDERS);
  });

  it("stays closed until the field is clicked, so tabbing through can't arm the backdrop", () => {
    const { input } = combo();
    fireEvent.focus(input);
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("narrows the suggestions to what has been typed", () => {
    const { input } = combo({ value: "des" });
    fireEvent.click(input);
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(["Design"]);
  });

  it("offers nothing once the field already spells one folder exactly", () => {
    const { input } = combo({ value: "Review" });
    fireEvent.click(input);
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("commits the clicked folder and closes", () => {
    const { input, onChange } = combo();
    fireEvent.click(input);
    fireEvent.click(screen.getByRole("option", { name: "Review" }));
    expect(onChange).toHaveBeenCalledWith("Review");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("arrows into the list and commits on Enter", () => {
    const { input, onChange } = combo();
    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onChange).toHaveBeenCalledWith("Design");
  });

  it("leaves a bare Enter to the surrounding dialog", () => {
    // Typing a folder that doesn't exist yet and pressing Enter has to submit,
    // not get swallowed by a list the user never arrowed into.
    const { input, onChange } = combo({ value: "Brand new" });
    fireEvent.click(input);
    const enter = fireEvent.keyDown(input, { key: "Enter" });
    expect(enter).toBe(true);
    expect(onChange).not.toHaveBeenCalled();
  });
});
