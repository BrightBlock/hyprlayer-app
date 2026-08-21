// @vitest-environment jsdom
import { useRef } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ESCAPE_COMMAND_ID, isRegistered, runCommand } from "@/lib/keybindings/registry";
import {
  Button,
  Card,
  ConfirmDialog,
  ContextMenuItem,
  Dialog,
  DropdownSelect,
  Field,
  IconButton,
  MenuItem,
  Popover,
  rowClass,
  type DropdownOption,
} from "./ui";

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
    // narrow settings-row trigger clipped its options to "Opus 4.…", something
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
    // `w-56` regardless of class order, the component has to drop its own
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

describe("cx / class merge", () => {
  // The exact regression: a plain `filter().join(" ")` couldn't resolve a
  // conflict between `Dialog`'s own `max-w-md` and a caller's width class,
  // and Tailwind sorts utilities into the stylesheet by scale name, so the
  // base always won regardless of which was written last. Five dialogs
  // shipped 448px wide because of it.
  it("lets Dialog's size prop win over its own max-w-md base", () => {
    render(
      <Dialog open onClose={vi.fn()} title="Wide" size="2xl">
        Body
      </Dialog>,
    );
    const panel = screen.getByRole("dialog");
    expect(panel.className).toMatch(/(^|\s)max-w-2xl(\s|$)/);
    expect(panel.className).not.toMatch(/(^|\s)max-w-md(\s|$)/);
  });

  it("lets an arbitrary-value className win over max-w-md", () => {
    render(
      <Dialog open onClose={vi.fn()} className="max-w-[940px]">
        Body
      </Dialog>,
    );
    const panel = screen.getByRole("dialog");
    expect(panel.className).toContain("max-w-[940px]");
    expect(panel.className).not.toMatch(/(^|\s)max-w-md(\s|$)/);
  });

  it("keeps a Card's non-conflicting className override and drops nothing else", () => {
    render(<Card data-testid="card" className="py-12" />);
    const card = screen.getByTestId("card");
    expect(card.className).toMatch(/(^|\s)py-12(\s|$)/);
    expect(card.className).toContain("rounded-lg");
    expect(card.className).toContain("border-edge");
  });

  it("merges a conflicting custom-token colour class, proving twMerge groups the app's theme colours", () => {
    // Card's base carries `bg-panel`; a caller's `bg-panel-2` is a same-group
    // conflict, not an unrelated utility, so the base must not survive too.
    render(<Card data-testid="card" className="bg-panel-2" />);
    const card = screen.getByTestId("card");
    expect(card.className).toMatch(/(^|\s)bg-panel-2(\s|$)/);
    expect(card.className).not.toMatch(/(^|\s)bg-panel(\s|$)/);
  });
});

describe("Dialog composition", () => {
  it("renders the footer outside the scrolling body", () => {
    render(
      <Dialog open onClose={vi.fn()} title="Composed" footer={<button>Done</button>}>
        Body
      </Dialog>,
    );
    const body = screen.getByText("Body");
    const footerButton = screen.getByRole("button", { name: "Done" });
    // The footer sits in its own band, a sibling of the body, not inside it.
    expect(body.parentElement).not.toBe(footerButton.parentElement);
    expect(footerButton.parentElement?.contains(body)).toBe(false);
  });

  it("adds a header rule when seams is set", () => {
    render(
      <Dialog open onClose={vi.fn()} title="Seams" seams>
        Body
      </Dialog>,
    );
    const header = screen.getByText("Seams").parentElement;
    expect(header?.className).toContain("border-b");
  });

  it("aligns near the top of the viewport when align is top", () => {
    const { container } = render(
      <Dialog open onClose={vi.fn()} align="top">
        Body
      </Dialog>,
    );
    const overlay = container.firstElementChild as HTMLElement;
    expect(overlay.className).toMatch(/(^|\s)items-start(\s|$)/);
    expect(overlay.className).not.toMatch(/(^|\s)items-center(\s|$)/);
  });
});

describe("ConfirmDialog", () => {
  it("puts Cancel before the confirm button in DOM order", () => {
    render(
      <ConfirmDialog
        open
        onClose={vi.fn()}
        title="Archive workstream?"
        confirmLabel="Archive"
        onConfirm={vi.fn()}
      >
        Body
      </ConfirmDialog>,
    );
    const buttons = screen.getAllByRole("button").map((b) => b.textContent);
    expect(buttons.indexOf("Cancel")).toBeLessThan(buttons.indexOf("Archive"));
  });

  it("disables both Cancel and the confirm button while busy", () => {
    render(
      <ConfirmDialog
        open
        onClose={vi.fn()}
        title="Archive workstream?"
        confirmLabel="Archive"
        onConfirm={vi.fn()}
        busy
      >
        Body
      </ConfirmDialog>,
    );
    const buttons = screen.getAllByRole("button") as HTMLButtonElement[];
    expect(buttons).toHaveLength(2);
    for (const button of buttons) expect(button.disabled).toBe(true);
  });

  it("leaves the confirm button unfocused by default", () => {
    render(
      <ConfirmDialog
        open
        onClose={vi.fn()}
        title="Archive workstream?"
        confirmLabel="Archive"
        onConfirm={vi.fn()}
      >
        Body
      </ConfirmDialog>,
    );
    expect(document.activeElement).not.toBe(screen.getByRole("button", { name: "Archive" }));
  });

  it("focuses the confirm button under autoFocusConfirm, so Enter commits", () => {
    render(
      <ConfirmDialog
        open
        onClose={vi.fn()}
        title="Quit Hyprlayer?"
        confirmLabel="Quit"
        onConfirm={vi.fn()}
        autoFocusConfirm
      >
        Body
      </ConfirmDialog>,
    );
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Quit" }));
  });
});

describe("rowClass", () => {
  it("emits each tier's height class", () => {
    expect(rowClass({ tier: "compact" })).toMatch(/(^|\s)h-7(\s|$)/);
    expect(rowClass({ tier: "standard" })).toMatch(/(^|\s)h-9(\s|$)/);
  });

  it("emits relative for a spine-selected row", () => {
    // Always true (rowClass emits `relative` unconditionally, RowSpine needs
    // it to position itself), but a spine row is the case that depends on it.
    expect(rowClass({ tier: "standard", selected: "spine" })).toMatch(/(^|\s)relative(\s|$)/);
  });

  it("emits no hover class when interactive is false", () => {
    expect(rowClass({ tier: "standard", interactive: false })).not.toMatch(/hover:/);
  });

  it("lets a caller's height class win over the tier's own", () => {
    // Same regression the Phase 1 tests guard: a plain join couldn't resolve
    // this, twMerge does.
    const cls = rowClass({ tier: "compact", className: "h-20" });
    expect(cls).toMatch(/(^|\s)h-20(\s|$)/);
    expect(cls).not.toMatch(/(^|\s)h-7(\s|$)/);
  });
});

describe("focus-visible rings", () => {
  it("Button carries the shared focus ring", () => {
    render(<Button>Click me</Button>);
    expect(screen.getByRole("button", { name: "Click me" }).className).toContain(
      "focus-visible:ring-2",
    );
  });

  it("IconButton carries the shared focus ring", () => {
    render(<IconButton aria-label="Icon action" />);
    expect(screen.getByRole("button", { name: "Icon action" }).className).toContain(
      "focus-visible:ring-2",
    );
  });

  it("ContextMenuItem carries the shared focus ring", () => {
    render(<ContextMenuItem>Item</ContextMenuItem>);
    expect(screen.getByRole("menuitem", { name: "Item" }).className).toContain(
      "focus-visible:ring-2",
    );
  });
});

function PopoverHarness({ open, onClose }: { open: boolean; onClose: () => void }) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  return (
    <div>
      <button ref={triggerRef}>Trigger</button>
      <Popover open={open} onClose={onClose} anchorRef={triggerRef}>
        <MenuItem>Item</MenuItem>
      </Popover>
    </div>
  );
}

describe("Popover", () => {
  it("closes on Escape via the shared registry", () => {
    const onClose = vi.fn();
    render(<PopoverHarness open onClose={onClose} />);
    expect(runCommand(ESCAPE_COMMAND_ID)).toBe(true);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closes on a backdrop click", () => {
    const onClose = vi.fn();
    render(<PopoverHarness open onClose={onClose} />);
    const backdrop = document.body.querySelector('div.fixed.inset-0[aria-hidden="true"]');
    expect(backdrop).not.toBeNull();
    fireEvent.click(backdrop!);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("unregisters its Escape handler once closed", () => {
    const { rerender } = render(<PopoverHarness open onClose={vi.fn()} />);
    expect(isRegistered(ESCAPE_COMMAND_ID)).toBe(true);
    rerender(<PopoverHarness open={false} onClose={vi.fn()} />);
    expect(isRegistered(ESCAPE_COMMAND_ID)).toBe(false);
  });

  it("nested inside a Dialog, closes only itself and leaves the dialog open", () => {
    // The dialog mounts (and registers its own Escape handler) first; the
    // popover opens on a later render, the same order a real trigger click
    // would produce — landing its handler on top of the LIFO stack.
    const closeDialog = vi.fn();
    const closePopover = vi.fn();
    function Nested({ popoverOpen }: { popoverOpen: boolean }) {
      const triggerRef = useRef<HTMLButtonElement>(null);
      return (
        <Dialog open onClose={closeDialog} title="Outer">
          <button ref={triggerRef}>Trigger</button>
          <Popover open={popoverOpen} onClose={closePopover} anchorRef={triggerRef}>
            <MenuItem>Item</MenuItem>
          </Popover>
        </Dialog>
      );
    }
    const { rerender } = render(<Nested popoverOpen={false} />);
    rerender(<Nested popoverOpen={true} />);

    expect(runCommand(ESCAPE_COMMAND_ID)).toBe(true);
    expect(closePopover).toHaveBeenCalledTimes(1);
    expect(closeDialog).not.toHaveBeenCalled();
  });
});
