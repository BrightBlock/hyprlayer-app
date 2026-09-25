import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

import { ChevronDownIcon, ChevronRightIcon } from "./icons";
import { MenuItem, MenuSeparator } from "./ui";

export type OptionMenuItem = {
  key: string;
  label: string;
  hint?: string;
  icon?: ReactNode;
  danger?: boolean;
  disabled?: boolean;
  disabledReason?: string;
  accent?: boolean;
};

export type OptionMenuGroup = {
  key: string;
  label: string;
  icon?: ReactNode;
  items: OptionMenuItem[];
  disabled?: boolean;
  more?: { label: string; items: OptionMenuItem[] };
};

const ITEM = '[role^="menuitem"]:not([disabled])';
const LOCKED_REASON =
  "Locked to this session's provider. Start a new agent to use a different provider";
const HEADING = "flex items-center gap-2 px-2.5 py-1 text-ui-sm font-medium text-ink-dim/70";

/** A menu body: focus starts on the checked (else first) item, arrows move it, and it returns on close. */
export function MenuPanel({
  children,
  "aria-label": ariaLabel,
}: {
  children: ReactNode;
  "aria-label"?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const panel = ref.current;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const target =
      panel?.querySelector<HTMLElement>('[aria-checked="true"]:not([disabled])') ??
      panel?.querySelector<HTMLElement>(ITEM);
    target?.focus({ preventScroll: true });
    target?.scrollIntoView?.({ block: "nearest" });
    return () => {
      const active = document.activeElement;
      // Only focus the closing menu stranded; an action that moved it keeps it.
      const stranded = !active || active === document.body || (panel?.contains(active) ?? false);
      if (stranded && previous?.isConnected) previous.focus({ preventScroll: true });
    };
  }, []);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    // Leaves a text field inside the menu its own Home/End.
    if (!target.matches('[role^="menuitem"]')) return;
    const items = Array.from(ref.current?.querySelectorAll<HTMLElement>(ITEM) ?? []);
    const from = items.indexOf(target);
    const next =
      e.key === "ArrowDown"
        ? from + 1
        : e.key === "ArrowUp"
          ? from - 1
          : e.key === "Home"
            ? 0
            : e.key === "End"
              ? items.length - 1
              : null;
    if (next === null || items.length === 0) return;
    e.preventDefault();
    const item = items[Math.min(items.length - 1, Math.max(0, next))];
    item.focus({ preventScroll: true });
    item.scrollIntoView?.({ block: "nearest" });
  };

  return (
    <div ref={ref} role="menu" aria-label={ariaLabel} onKeyDown={onKeyDown}>
      {children}
    </div>
  );
}

/** One radio set of options, flat or grouped; the caller's `onSelect` closes the menu. */
export function OptionMenuList({
  items = [],
  groups,
  activeKey,
  onSelect,
  heading,
  "aria-label": ariaLabel,
}: {
  items?: OptionMenuItem[];
  groups?: OptionMenuGroup[];
  activeKey: string;
  onSelect: (key: string) => void;
  /** Shown over flat `items`; also names the set. */
  heading?: string;
  "aria-label"?: string;
}) {
  const [expandedGroups, setExpandedGroups] = useState<ReadonlySet<string>>(new Set());
  const radio = (key: string) => ({
    role: "menuitemradio" as const,
    "aria-checked": key === activeKey,
  });

  return (
    <div role="group" aria-label={ariaLabel ?? heading}>
      {groups ? (
        groups.map((g, gi) => {
          const renderItem = (it: OptionMenuItem) =>
            g.disabled || it.disabled ? (
              <MenuItem
                key={it.key}
                {...radio(it.key)}
                disabled
                title={g.disabled ? undefined : (it.disabledReason ?? LOCKED_REASON)}
              >
                <span className="flex w-full items-center gap-2 text-ink-dim/40">
                  {it.icon && <span className="shrink-0">{it.icon}</span>}
                  <span className="min-w-0 flex-1 truncate">{it.label}</span>
                  {it.hint && <span className="shrink-0 text-ui-sm text-ink-dim/70">{it.hint}</span>}
                  {g.disabled && (
                    <span className="ml-auto shrink-0 text-label">soon</span>
                  )}
                </span>
              </MenuItem>
            ) : (
              <MenuItem
                key={it.key}
                {...radio(it.key)}
                onClick={() => onSelect(it.key)}
                className={it.key === activeKey ? "text-ink" : "text-ink-dim"}
              >
                <span className="flex w-full items-center gap-2">
                  {it.icon && <span className="shrink-0">{it.icon}</span>}
                  <span className="min-w-0 flex-1 truncate">{it.label}</span>
                  {it.hint && <span className="shrink-0 text-ink-dim">{it.hint}</span>}
                  {it.key === activeKey && (
                    <span aria-hidden="true" className="ml-auto shrink-0">
                      ✓
                    </span>
                  )}
                </span>
              </MenuItem>
            );
          const showMore =
            g.more && (expandedGroups.has(g.key) || g.more.items.some((it) => it.key === activeKey));
          return (
            <div key={g.key}>
              {gi > 0 && <MenuSeparator />}
              <div role="presentation" className={HEADING}>
                {g.icon && <span className="shrink-0">{g.icon}</span>}
                <span>{g.label}</span>
              </div>
              {g.items.map(renderItem)}
              {g.more && g.more.items.length > 0 && (
                <>
                  <MenuItem
                    onClick={() =>
                      setExpandedGroups((prev) => {
                        const next = new Set(prev);
                        if (!next.delete(g.key)) next.add(g.key);
                        return next;
                      })
                    }
                    className="text-ink-dim"
                  >
                    <span className="flex w-full items-center gap-2">
                      <span className="shrink-0">
                        {showMore ? <ChevronDownIcon size={14} /> : <ChevronRightIcon size={14} />}
                      </span>
                      <span className="min-w-0 flex-1 truncate">{g.more.label}</span>
                      <span className="ml-auto shrink-0 text-ui-sm text-ink-dim/70">
                        {g.more.items.length}
                      </span>
                    </span>
                  </MenuItem>
                  {showMore && g.more.items.map(renderItem)}
                </>
              )}
            </div>
          );
        })
      ) : (
        <>
          {heading && (
            <div role="presentation" className={HEADING}>
              {heading}
            </div>
          )}
          {items.map((it) =>
            it.disabled ? (
              <MenuItem key={it.key} {...radio(it.key)} disabled title={it.hint || undefined}>
                <span className="flex w-full items-center gap-2 text-ink-dim/40">
                  {it.icon && <span className="shrink-0">{it.icon}</span>}
                  <span>{it.label}</span>
                  {it.hint && <span>{it.hint}</span>}
                </span>
              </MenuItem>
            ) : (
              <MenuItem
                key={it.key}
                {...radio(it.key)}
                onClick={() => onSelect(it.key)}
                danger={it.danger}
                className={
                  it.danger
                    ? undefined
                    : it.accent
                      ? "text-accent hover:text-accent"
                      : it.key === activeKey
                        ? "text-ink"
                        : "text-ink-dim"
                }
              >
                <span className="flex w-full items-center gap-2">
                  {it.icon && <span className="shrink-0">{it.icon}</span>}
                  <span>{it.label}</span>
                  {it.hint && <span className="text-ink-dim">{it.hint}</span>}
                  {it.key === activeKey && (
                    <span aria-hidden="true" className="ml-auto">
                      ✓
                    </span>
                  )}
                </span>
              </MenuItem>
            ),
          )}
        </>
      )}
    </div>
  );
}
