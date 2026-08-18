/** Small controls the Agents list and editor share. */

import type { ReactNode } from "react";

import { TextInput } from "@/components/ui";

export type SegmentedOption<T extends string> = {
  key: T;
  label: ReactNode;
  /** Renders the label monospaced — used for the project-scoped segment. */
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
 * plugin's `overlay.folder`) is free text — a user or plugin author can file
 * an agent under a folder that isn't a derived phase — so this is a text
 * input offering the known folders as a `<datalist>`, not a closed
 * `DropdownSelect`. Used by both the editor's Folder field and the bulk move
 * dialog's target picker.
 */
export function FolderCombo({
  id,
  value,
  onChange,
  folders,
  placeholder,
  "aria-label": ariaLabel,
}: {
  /** Unique per instance — backs the `<datalist>` id the input points at. */
  id: string;
  value: string;
  onChange: (value: string) => void;
  folders: readonly string[];
  placeholder?: string;
  "aria-label"?: string;
}) {
  const listId = `${id}-folders`;
  return (
    <>
      <TextInput
        list={listId}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        aria-label={ariaLabel}
      />
      <datalist id={listId}>
        {folders.map((f) => (
          <option key={f} value={f} />
        ))}
      </datalist>
    </>
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
          {hint && <span className="font-mono text-[11px] text-ink-dim">{hint}</span>}
        </span>
        {description && <span className="mt-0.5 block text-xs text-ink-dim">{description}</span>}
      </span>
    </label>
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
      <h2 className="text-[11px] font-semibold uppercase tracking-wider text-ink-dim">{label}</h2>
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

/** An inline note — the system-agent explainer and the bypass-permissions
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
