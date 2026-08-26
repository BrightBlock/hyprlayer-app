// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { BranchPrefix } from "@/lib/domain/branchPrefix";
import { BranchPrefixEditor } from "./BranchPrefixEditor";

afterEach(cleanup);

const TWO: BranchPrefix[] = [
  { label: "Bug fix", prefix: "fix/", keywords: ["fix", "bug"] },
  { label: "Feature", prefix: "feat/", keywords: ["feature", "add"] },
];

function renderEditor(
  props: Partial<Parameters<typeof BranchPrefixEditor>[0]> = {},
) {
  const onChange = vi.fn();
  const view = render(
    <BranchPrefixEditor
      title={<h2>Branch types</h2>}
      value={[]}
      onChange={onChange}
      inheritedNote="Built-in types"
      {...props}
    />,
  );
  return { onChange, view };
}

const labels = () => screen.getAllByLabelText("Label") as HTMLInputElement[];
const prefixes = () => screen.getAllByLabelText("Prefix") as HTMLInputElement[];
const keywords = () =>
  screen.getAllByLabelText("Keywords") as HTMLInputElement[];
const addType = () => screen.getByRole("button", { name: "Add branch type" });
const confirmAdd = () => screen.getByRole("button", { name: "Add" });
const cancelAdd = () => screen.getByRole("button", { name: "Cancel add" });

function editRow(name: string) {
  fireEvent.click(screen.getByRole("button", { name: `Edit ${name}` }));
}

function type(input: HTMLInputElement, text: string) {
  fireEvent.change(input, { target: { value: text } });
}

describe("BranchPrefixEditor", () => {
  it("opens an add form without touching the list", () => {
    const { onChange } = renderEditor();
    fireEvent.click(addType());

    expect(labels()).toHaveLength(1);
    expect(labels()[0].value).toBe("");
    expect((confirmAdd() as HTMLButtonElement).disabled).toBe(true);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("disables Add while a field is still blank, without an error message", () => {
    renderEditor();
    fireEvent.click(addType());

    type(labels()[0], "Spike");

    expect((confirmAdd() as HTMLButtonElement).disabled).toBe(true);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("disables Add and explains why once the prefix is invalid", () => {
    renderEditor();
    fireEvent.click(addType());

    type(labels()[0], "Spike");
    type(prefixes()[0], "-fix/");

    expect((confirmAdd() as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole("alert").textContent).toContain(
      "must not start with '-'",
    );
  });

  it("disables Add when the prefix duplicates an existing type's", () => {
    const { onChange } = renderEditor({ value: TWO });
    fireEvent.click(addType());

    type(labels()[0], "Also a fix");
    type(prefixes()[0], "fix/");

    expect((confirmAdd() as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole("alert").textContent).toMatch(/already uses/);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("confirms a valid new type and adds it to the table", () => {
    const { onChange } = renderEditor();
    fireEvent.click(addType());

    type(labels()[0], "Spike");
    type(prefixes()[0], "spike/");
    fireEvent.click(confirmAdd());

    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith([
      { label: "Spike", prefix: "spike/", keywords: [] },
    ]);
    expect(screen.queryByRole("button", { name: "Cancel add" })).toBeNull();
  });

  it("renders a just-added row as text until its edit icon is clicked", () => {
    renderEditor();
    fireEvent.click(addType());
    type(labels()[0], "Spike");
    type(prefixes()[0], "spike/");
    fireEvent.click(confirmAdd());

    expect(screen.queryByLabelText("Label")).toBeNull();
    expect(screen.getByText("Spike")).toBeTruthy();
    expect(screen.getByText("spike/")).toBeTruthy();

    editRow("Spike");

    expect(labels()).toHaveLength(1);
    expect(labels()[0].value).toBe("Spike");
  });

  it("renders SAVED rows as text on a fresh mount, the way a restart arrives", () => {
    renderEditor({ value: TWO });

    expect(screen.queryByLabelText("Label")).toBeNull();
    expect(screen.queryByLabelText("Prefix")).toBeNull();
    expect(screen.getByText("fix/")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Edit Bug fix" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Edit Feature" })).toBeTruthy();
  });

  it("returns an edited row to text once its edit is valid and committed", () => {
    const { onChange } = renderEditor({ value: TWO });

    editRow("Bug fix");
    type(prefixes()[0], "bugfix/");
    fireEvent.keyDown(prefixes()[0], { key: "Enter" });

    expect(onChange).toHaveBeenCalledWith([
      { ...TWO[0], prefix: "bugfix/" },
      TWO[1],
    ]);
    expect(screen.queryByLabelText("Prefix")).toBeNull();
    expect(screen.getByText("bugfix/")).toBeTruthy();
  });

  it("keeps an invalid row in edit mode so its message stays visible", () => {
    const { onChange } = renderEditor({ value: TWO });

    editRow("Bug fix");
    type(prefixes()[0], "-bad/");
    fireEvent.blur(prefixes()[0]);

    expect(onChange).not.toHaveBeenCalled();
    expect(prefixes()).toHaveLength(1);
    expect(screen.getByRole("alert").textContent).toContain(
      "must not start with '-'",
    );
  });

  it("commits a valid new type on Enter, the same as clicking Add", () => {
    const { onChange } = renderEditor();
    fireEvent.click(addType());

    type(labels()[0], "Spike");
    type(prefixes()[0], "spike/");
    fireEvent.keyDown(prefixes()[0], { key: "Enter" });

    expect(onChange).toHaveBeenCalledWith([
      { label: "Spike", prefix: "spike/", keywords: [] },
    ]);
  });

  it("cancels the add form and discards its draft", () => {
    const { onChange } = renderEditor();
    fireEvent.click(addType());

    type(labels()[0], "Spike");
    fireEvent.click(cancelAdd());

    expect(onChange).not.toHaveBeenCalled();
    expect(screen.queryByLabelText("Label")).toBeNull();
    expect(screen.getByText("Built-in types")).toBeTruthy();
  });

  it("closes the add form on Escape without adding a row", () => {
    const { onChange } = renderEditor();
    fireEvent.click(addType());

    type(labels()[0], "Spike");
    fireEvent.keyDown(labels()[0], { key: "Escape" });

    expect(onChange).not.toHaveBeenCalled();
    expect(screen.queryByLabelText("Label")).toBeNull();
  });

  it("blocks the save and marks the field when a saved row is blanked", () => {
    const { onChange } = renderEditor({ value: TWO });

    editRow("Bug fix");
    type(prefixes()[0], "");
    fireEvent.blur(prefixes()[0]);

    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain("needs a prefix");
    expect(labels()).toHaveLength(1);
    expect(labels()[0].value).toBe("Bug fix");
    expect(screen.getByText("Feature")).toBeTruthy();
  });

  it("marks an invalid prefix inline rather than round-tripping a save", () => {
    const { onChange } = renderEditor({ value: TWO });

    editRow("Bug fix");
    type(prefixes()[0], "-fix/");
    fireEvent.blur(prefixes()[0]);

    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain(
      "must not start with '-'",
    );
  });

  it("removes a row and commits the rest", () => {
    const { onChange } = renderEditor({ value: TWO });

    fireEvent.click(screen.getByRole("button", { name: "Remove Bug fix" }));

    expect(onChange).toHaveBeenCalledWith([TWO[1]]);
  });

  it("reorders with the down arrow and commits the swap", () => {
    const { onChange } = renderEditor({ value: TWO });

    fireEvent.click(screen.getByRole("button", { name: "Move Bug fix down" }));

    expect(onChange).toHaveBeenCalledWith([TWO[1], TWO[0]]);
  });

  it("drafts text edits and commits them on blur, not per keystroke", () => {
    const { onChange } = renderEditor({ value: TWO });

    editRow("Bug fix");
    type(prefixes()[0], "bugfix/");
    expect(onChange).not.toHaveBeenCalled();

    fireEvent.blur(prefixes()[0]);
    expect(onChange).toHaveBeenCalledWith([
      { ...TWO[0], prefix: "bugfix/" },
      TWO[1],
    ]);
  });

  it("parses the comma-separated keywords cell into a list", () => {
    const { onChange } = renderEditor({ value: [TWO[0]] });

    editRow("Bug fix");
    type(keywords()[0], "fix, bug ,crash");
    fireEvent.blur(keywords()[0]);

    expect(onChange).toHaveBeenCalledWith([
      { label: "Bug fix", prefix: "fix/", keywords: ["fix", "bug", "crash"] },
    ]);
  });

  it("renders the inherited note instead of rows when the list is empty", () => {
    renderEditor({ inheritedNote: "Your global types" });

    expect(screen.getByText("Your global types")).toBeTruthy();
    expect(screen.queryByLabelText("Label")).toBeNull();
  });

  it("labels the columns once there's a row to label, not before", () => {
    const { rerender } = render(
      <BranchPrefixEditor
        title={<h2>Branch types</h2>}
        value={[]}
        onChange={vi.fn()}
        inheritedNote="Built-in types"
      />,
    );
    expect(screen.queryByText("Label")).toBeNull();

    rerender(
      <BranchPrefixEditor
        title={<h2>Branch types</h2>}
        value={TWO}
        onChange={vi.fn()}
        inheritedNote="Built-in types"
      />,
    );
    expect(screen.getByText("Label")).toBeTruthy();
    expect(screen.getByText("Prefix")).toBeTruthy();
    expect(screen.getByText("Keywords")).toBeTruthy();
  });

  it("disables every row control", () => {
    renderEditor({ value: TWO, disabled: true });

    for (const name of [
      "Edit Bug fix",
      "Move Bug fix down",
      "Remove Bug fix",
      "Move Feature up",
      "Add branch type",
    ]) {
      expect(
        (screen.getByRole("button", { name }) as HTMLButtonElement).disabled,
      ).toBe(true);
    }
  });

  it("re-seeds from a changed value, unless the draft is blocked", () => {
    const onChange = vi.fn();
    const { rerender } = render(
      <BranchPrefixEditor
        title={<h2>Branch types</h2>}
        value={TWO}
        onChange={onChange}
        blockReseed
        inheritedNote="Built-in types"
      />,
    );
    editRow("Bug fix");
    type(prefixes()[0], "my fix/");

    rerender(
      <BranchPrefixEditor
        title={<h2>Branch types</h2>}
        value={[TWO[1]]}
        onChange={onChange}
        blockReseed
        inheritedNote="Built-in types"
      />,
    );
    expect(prefixes()[0].value).toBe("my fix/");
    expect(screen.getByText("Feature")).toBeTruthy();

    rerender(
      <BranchPrefixEditor
        title={<h2>Branch types</h2>}
        value={[TWO[1]]}
        onChange={onChange}
        inheritedNote="Built-in types"
      />,
    );
    expect(screen.queryByLabelText("Prefix")).toBeNull();
    expect(screen.getByText("feat/")).toBeTruthy();
  });
});
