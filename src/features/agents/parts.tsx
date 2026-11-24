
import { useRef, useState, type KeyboardEvent, type ReactNode } from "react";

import { MenuItem, Popover, TextInput } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { folderColor } from "./agents";

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
                <Icon name="folder" size="sm" />
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

