/** Small controls the Agents list and editor share. */

import { useRef, useState, type KeyboardEvent, type ReactNode } from "react";

import { MenuItem, Popover, TextInput } from "@/components/ui";
import { FolderIcon } from "@/components/icons";
import { folderColor } from "./agents";

export type SegmentedOption<T extends string> = {
  key: T;
  label: ReactNode;
  /** Renders the label monospaced, used for the project-scoped segment. */
  mono?: boolean;
};

/** A pill-grouped exclusive choice: the list's Your/System and scope switches,
 *  and the editor's tab strip, effort and harness pickers. */
export function Segmented<T extends string>({
  options,
  value,
  onChange,
  size = "md",
  disabled,
  "aria-label": ariaLabel,
}: {
  options: ReadonlyArray<SegmentedOption<T>>;
  value: T;
  onChange: (next: T) => void;
  size?: "sm" | "md";
  disabled?: boolean;
  "aria-label"?: string;
}) {
  return (
    <div
      role="group"
      aria-label={ariaLabel}
      className="inline-flex gap-0.5 rounded-lg bg-panel-2 p-0.5 ring-1 ring-inset ring-edge"
    >
      {options.map((o) => {
        const active = o.key === value;
        return (
          <button
            key={o.key}
            type="button"
            aria-pressed={active}
            disabled={disabled}
            onClick={() => onChange(o.key)}
            className={[
              "rounded-md font-medium transition-colors disabled:cursor-default",
              size === "sm" ? "px-2.5 py-1 text-xs" : "px-3 py-1 text-sm",
              o.mono ? "font-mono" : "",
              active ? "bg-panel text-ink" : "text-ink-dim enabled:hover:text-ink",
            ].join(" ")}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

/**
 * A free-text folder field with autocomplete suggestions. `category` (and a
 * plugin's `overlay.folder`) is free text, a user or plugin author can file
 * an agent under a folder that isn't a derived phase, so this is a text
 * input offering the known folders, not a closed `DropdownSelect`. Used by
 * both the editor's Folder field and the move dialog's target picker.
 *
 * The suggestions used to be a native `<datalist>`. The dropdown a datalist
 * draws is the platform's, not the app's: it takes no theme token, no radius
 * and no type scale, so inside the Move dialog it rendered as a bare system
 * list against the dialog's own panel. `Popover` gets the app's styling, and
 * (ADR-0006 "Floating panels") the Escape handling, the `z-50` and the portal
 * that keeps the panel from being clipped by the dialog body's own scroll.
 */
export function FolderCombo({
  id,
  value,
  onChange,
  folders,
  placeholder,
  "aria-label": ariaLabel,
}: {
  /** Unique per instance, backs the suggestion listbox's id. */
  id: string;
  value: string;
  onChange: (value: string) => void;
  folders: readonly string[];
  placeholder?: string;
  "aria-label"?: string;
}) {
  const [open, setOpen] = useState(false);
  /** Index into `suggestions`, or -1 for "the user hasn't arrowed in yet". */
  const [active, setActive] = useState(-1);
  const anchorRef = useRef<HTMLDivElement>(null);
  const listId = `${id}-folders`;

  const q = value.trim().toLowerCase();
  const matches = folders.filter((f) => f.toLowerCase().includes(q));
  // An exact match means the field already says what the only row would say.
  const suggestions = matches.length === 1 && matches[0].toLowerCase() === q ? [] : matches;

  const close = () => {
    setOpen(false);
    setActive(-1);
  };
  const commit = (folder: string) => {
    onChange(folder);
    close();
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      if (suggestions.length === 0) return;
      e.preventDefault();
      const step = e.key === "ArrowDown" ? 1 : -1;
      if (!open) {
        setOpen(true);
        setActive(step === 1 ? 0 : suggestions.length - 1);
        return;
      }
      setActive((i) => (i + step + suggestions.length) % suggestions.length);
      return;
    }
    // Only with a row highlighted. A bare Enter belongs to the dialog's own
    // primary action — swallowing it would break "type a new name, press
    // Enter", which is the whole reason this is a combobox and not a listbox.
    if (e.key === "Enter" && open && active >= 0 && active < suggestions.length) {
      e.preventDefault();
      commit(suggestions[active]);
    }
  };

  return (
    <div ref={anchorRef}>
      <TextInput
        role="combobox"
        aria-expanded={open && suggestions.length > 0}
        aria-controls={listId}
        aria-autocomplete="list"
        // The browser's own history dropdown would cover the suggestions with
        // a second, unstyled panel saying the same names back.
        autoComplete="off"
        value={value}
        onChange={(e) => {
          onChange(e.target.value);
          setActive(-1);
          setOpen(true);
        }}
        // Click, not focus: the panel comes with a full-viewport backdrop that
        // swallows the next click anywhere, so tabbing through the editor's
        // Folder field must not arm it.
        onClick={() => setOpen(true)}
        onKeyDown={onKeyDown}
        placeholder={placeholder}
        aria-label={ariaLabel}
      />
      <Popover
        open={open && suggestions.length > 0}
        onClose={close}
        anchorRef={anchorRef}
        matchAnchorWidth
        className="max-h-56 overflow-y-auto"
      >
        <div id={listId} role="listbox" aria-label={ariaLabel}>
          {suggestions.map((f, i) => (
            <MenuItem
              key={f}
              role="option"
              aria-selected={f === value}
              // Pointer and keyboard drive one highlight, so arrowing down and
              // then moving the mouse can't leave two rows looking chosen.
              onMouseEnter={() => setActive(i)}
              onClick={() => commit(f)}
              className={`flex items-center gap-2 ${i === active ? "bg-panel-2" : ""}`}
            >
              <span className="shrink-0" style={{ color: folderColor(f) }}>
                <FolderIcon size={13} />
              </span>
              <span className="min-w-0 flex-1 truncate">{f}</span>
            </MenuItem>
          ))}
        </div>
      </Popover>
    </div>
  );
}

/** A radio row: the ring + fill dot, a label, and an optional trailing hint or
 *  description block. Used by the model grid and the permission-mode list. */
export function RadioRow({
  checked,
  onSelect,
  label,
  hint,
  description,
  labelClassName,
  name,
  disabled,
}: {
  checked: boolean;
  onSelect: () => void;
  disabled?: boolean;
  label: ReactNode;
  /** Short trailing qualifier on the same line (e.g. a context window). */
  hint?: ReactNode;
  /** A second, muted line beneath the label. */
  description?: ReactNode;
  labelClassName?: string;
  /** Groups the radios for assistive tech. */
  name: string;
}) {
  return (
    <label
      className={[
        "flex items-start gap-2.5 rounded-md p-2 text-sm",
        disabled ? "cursor-default opacity-70" : "cursor-pointer",
        checked && description ? "bg-panel-2" : disabled ? "" : "hover:bg-panel-2",
      ].join(" ")}
    >
      <input
        type="radio"
        name={name}
        checked={checked}
        disabled={disabled}
        onChange={onSelect}
        className="sr-only"
      />
      <span
        aria-hidden="true"
        className={[
          "mt-0.5 grid h-3.5 w-3.5 shrink-0 place-items-center rounded-full border-[1.5px]",
          checked ? "border-accent" : "border-edge",
        ].join(" ")}
      >
        <span className={`h-1.5 w-1.5 rounded-full ${checked ? "bg-accent" : ""}`} />
      </span>
      <span className="min-w-0">
        <span className="flex items-baseline gap-2">
          <span className={labelClassName ?? "text-ink"}>{label}</span>
          {hint && <span className="font-mono text-2xs text-ink-dim">{hint}</span>}
        </span>
        {description && <span className="mt-0.5 block text-xs text-ink-dim">{description}</span>}
      </span>
    </label>
  );
}

/** The Skills placeholder's "not live yet" marker — the sidebar's root row
 *  and the content pane's heading both carry it, matching the same treatment
 *  the "proj" and "Overrides global in…" tags already use elsewhere in the
 *  app (`bg-brand/15` / `text-brand`), rather than inventing a new tone. */
export function SoonBadge() {
  return (
    <span className="shrink-0 rounded bg-brand/15 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-brand">
      Soon
    </span>
  );
}

/**
 * A labelled run of [`EditorCard`]s. The eyebrow names the concern the cards
 * share; the caller's wider gap between groups is what separates them, so a
 * tab of otherwise-identical cards reads as sections rather than one stack.
 */
export function CardGroup({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2.5">
      <h2 className="text-2xs font-semibold uppercase tracking-wider text-ink-dim">{label}</h2>
      <div className="flex flex-col gap-3.5">{children}</div>
    </section>
  );
}

/** A titled panel in the editor body. */
export function EditorCard({
  title,
  description,
  action,
  children,
  className,
  bare,
}: {
  title?: ReactNode;
  description?: ReactNode;
  /** Controls pinned to the title row's right edge. */
  action?: ReactNode;
  children?: ReactNode;
  className?: string;
  /** Drops the vertical inset so a divided row list runs to the card's edges. */
  bare?: boolean;
}) {
  return (
    <section
      className={`rounded-lg border border-edge bg-panel ${bare ? "px-4" : "p-4"} ${className ?? ""}`}
    >
      {(title || action) && (
        <div className="mb-3 flex items-start justify-between gap-3">
          <div className="min-w-0">
            {title && <h3 className="text-sm font-semibold text-ink">{title}</h3>}
            {description && <p className="mt-0.5 text-xs text-ink-dim">{description}</p>}
          </div>
          {action && <div className="flex shrink-0 gap-1.5">{action}</div>}
        </div>
      )}
      {children}
    </section>
  );
}

/** An inline note, the system-agent explainer and the bypass-permissions
 *  warning. `tone` picks the tint. */
export function Callout({
  tone,
  icon,
  children,
}: {
  tone: "warn" | "bad";
  icon: ReactNode;
  children: ReactNode;
}) {
  const tint = tone === "bad" ? "border-bad/35 bg-bad/10" : "border-warn/35 bg-warn/10";
  return (
    <div className={`flex gap-2.5 rounded-lg border p-3 text-xs text-ink-dim ${tint}`}>
      <span className={`shrink-0 pt-px ${tone === "bad" ? "text-bad" : "text-warn"}`}>{icon}</span>
      <span className="text-pretty">{children}</span>
    </div>
  );
}
