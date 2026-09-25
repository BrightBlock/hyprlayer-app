
import { useRef, useState, type KeyboardEvent, type ReactNode } from "react";

import { MenuItem, Popover, TextInput } from "@/components/ui";
import { FolderIcon } from "@/components/icons";
import { folderColor } from "./agents";

export type SegmentedOption<T extends string> = {
  key: T;
  label: ReactNode;
  mono?: boolean;
};

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
              size === "sm" ? "px-2.5 py-1 text-ui" : "px-3 py-1 text-body",
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

export function FolderCombo({
  id,
  value,
  onChange,
  folders,
  placeholder,
  "aria-label": ariaLabel,
}: {
  id: string;
  value: string;
  onChange: (value: string) => void;
  folders: readonly string[];
  placeholder?: string;
  "aria-label"?: string;
}) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const anchorRef = useRef<HTMLDivElement>(null);
  const listId = `${id}-folders`;

  const q = value.trim().toLowerCase();
  const matches = folders.filter((f) => f.toLowerCase().includes(q));
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
        autoComplete="off"
        value={value}
        onChange={(e) => {
          onChange(e.target.value);
          setActive(-1);
          setOpen(true);
        }}
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
  hint?: ReactNode;
  description?: ReactNode;
  labelClassName?: string;
  name: string;
}) {
  return (
    <label
      className={[
        "flex items-start gap-2.5 rounded-md p-2 text-body",
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
          {hint && <span className="text-meta text-ink-dim">{hint}</span>}
        </span>
        {description && <span className="mt-0.5 block text-ui text-ink-dim">{description}</span>}
      </span>
    </label>
  );
}

export function SoonBadge() {
  return (
    <span className="shrink-0 rounded bg-brand/15 px-1.5 py-0.5 text-tag text-brand">
      Soon
    </span>
  );
}

export function CardGroup({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2.5">
      <h2 className="text-label text-ink-dim">{label}</h2>
      <div className="flex flex-col gap-3.5">{children}</div>
    </section>
  );
}

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
  action?: ReactNode;
  children?: ReactNode;
  className?: string;
  bare?: boolean;
}) {
  return (
    <section
      className={`rounded-lg border border-edge bg-panel ${bare ? "px-4" : "p-4"} ${className ?? ""}`}
    >
      {(title || action) && (
        <div className="mb-3 flex items-start justify-between gap-3">
          <div className="min-w-0">
            {title && <h3 className="text-body font-semibold text-ink">{title}</h3>}
            {description && <p className="mt-0.5 text-ui text-ink-dim">{description}</p>}
          </div>
          {action && <div className="flex shrink-0 gap-1.5">{action}</div>}
        </div>
      )}
      {children}
    </section>
  );
}

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
    <div className={`flex gap-2.5 rounded-lg border p-3 text-ui text-ink-dim ${tint}`}>
      <span className={`shrink-0 pt-px ${tone === "bad" ? "text-bad" : "text-warn-ink"}`}>{icon}</span>
      <span className="text-pretty">{children}</span>
    </div>
  );
}
