import { Fragment, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type {
  ButtonHTMLAttributes,
  HTMLAttributes,
  InputHTMLAttributes,
  ReactNode,
} from "react";

import { errorMessage, errorRemedy } from "@/lib/ipc";
import { formatChord } from "@/lib/keybindings/chord";
import { useEscape } from "@/lib/keybindings/registry";
import { IS_MACOS } from "@/lib/platform";
import { ChevronDownIcon, RestartIcon, XIcon } from "./icons";

function cx(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(" ");
}

/**
 * A keyboard-chord hint (⌘⇧P / Ctrl+Shift+P), formatted for the host OS.
 * macOS concatenates the modifier glyphs with no separators, so they're tracked
 * apart to stay legible at hint sizes, the same treatment as Settings →
 * Keyboard's larger pill, and like it deliberately NOT `font-mono` (monospace
 * renders ⌘/⇧ heavy and oversized). Size, color, and background come from
 * `className` (unsized it inherits the surrounding text) so each surface picks
 * a scale that matches its labels while sharing the one glyph treatment.
 */
export function ChordHint({ chord, className }: { chord: string; className?: string }) {
  return (
    <kbd
      className={cx("whitespace-nowrap font-medium", className)}
      style={IS_MACOS ? { letterSpacing: "0.14em" } : undefined}
    >
      {formatChord(chord, IS_MACOS)}
    </kbd>
  );
}

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "ghost" | "danger";
  busy?: boolean;
};

export function Button({ variant = "ghost", busy, className, children, disabled, ...rest }: ButtonProps) {
  return (
    <button
      className={cx(
        "inline-flex items-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
        "disabled:cursor-not-allowed disabled:opacity-50",
        variant === "primary" && "border border-accent bg-accent text-accent-ink hover:bg-accent/90",
        variant === "ghost" && "border border-edge bg-panel-2 hover:bg-edge/60",
        variant === "danger" && "border border-bad/40 text-bad hover:bg-bad/10",
        className,
      )}
      disabled={disabled || busy}
      {...rest}
    >
      {busy && <Spinner size={14} />}
      {children}
    </button>
  );
}

export function Card({ className, ...rest }: HTMLAttributes<HTMLDivElement>) {
  return <div className={cx("rounded-lg border border-edge bg-panel p-4", className)} {...rest} />;
}

export function CardTitle({ className, ...rest }: HTMLAttributes<HTMLHeadingElement>) {
  return <h2 className={cx("mb-3 text-sm font-semibold tracking-wide text-ink", className)} {...rest} />;
}

export type BadgeTone = "ok" | "warn" | "bad" | "dim" | "accent";

export function Badge({ tone = "dim", children }: { tone?: BadgeTone; children: ReactNode }) {
  const tones: Record<BadgeTone, string> = {
    ok: "bg-ok/15 text-ok",
    warn: "bg-warn/15 text-warn",
    bad: "bg-bad/15 text-bad",
    dim: "bg-edge/60 text-ink-dim",
    accent: "border border-edge bg-panel-2 text-ink",
  };
  return (
    <span className={cx("inline-flex items-center rounded px-1.5 py-0.5 text-xs font-medium", tones[tone])}>
      {children}
    </span>
  );
}

export function Spinner({ size = 16 }: { size?: number }) {
  return (
    <svg
      className="animate-spin text-ink-dim"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      aria-label="Loading"
    >
      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
      <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v4a4 4 0 00-4 4H4z" />
    </svg>
  );
}

export function EmptyState({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 p-12 text-center">
      <p className="text-base font-medium text-ink">{title}</p>
      {children && <div className="max-w-md text-sm text-ink-dim">{children}</div>}
    </div>
  );
}

export function TextInput({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      className={cx(
        "selectable w-full rounded-md border border-edge bg-panel-2 px-2.5 py-1.5 text-sm text-ink",
        "placeholder:text-ink-dim/70 focus:border-ink-dim/70 focus:outline-none",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...rest}
    />
  );
}

/** Gap between the trigger and its menu, and the menu's minimum inset from any
 *  viewport edge. Shared by the open-position and clamp passes below. */
const GAP = 4;
const MARGIN = 8;

export type DropdownOption = {
  value: string;
  label: string;
  /** Rendered dimmed and unselectable, the `<option disabled>` equivalent. */
  disabled?: boolean;
  /** Heading to file this option under; runs of equal values share one header
   *  (the `<optgroup>` equivalent). Ungrouped options render bare. */
  group?: string;
};

type DropdownSelectProps = {
  value: string;
  options: DropdownOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
  className?: string;
  /** Names the trigger when no surrounding text does. */
  "aria-label"?: string;
  /** Trigger text when `value` matches no option, otherwise a stale/unknown
   *  value would leak onto the button as raw text. */
  placeholder?: string;
};

/**
 * The app's ONLY select control. Native `<select>` is deliberately absent: the
 * OS draws its open menu, so on Linux/WebKitGTK it renders as a white GTK popup
 * over a dark theme, and macOS/Windows ignore the palette too. This draws the
 * menu in the DOM, themed like everything else on every platform.
 *
 * Positioning is `fixed` off the trigger's rect, so overflow containers
 * (dialogs, sidebars) can't clip it. It caps to the viewport, scrolls, and
 * flips upward when there's more room above.
 *
 * The menu and backdrop render in a portal on `document.body`, never inline.
 * Inline they'd be descendants of `Field`'s `<label>`, and WebKit forwards
 * label-descendant clicks to the labeled control, so every click would
 * re-toggle the trigger. The portal also escapes a dialog overlay's
 * `backdrop-blur`, which makes `position: fixed` behave relatively in WebKit.
 *
 * Keyboard: ↓/↑/Enter/Space opens, ↓/↑/Home/End walk the enabled options,
 * Enter/Space picks, Escape closes only the menu.
 */
export function DropdownSelect({
  value,
  options,
  onChange,
  disabled,
  className,
  placeholder,
  "aria-label": ariaLabel,
}: DropdownSelectProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const [pos, setPos] = useState<{
    top?: number;
    bottom?: number;
    left: number;
    width: number;
    maxHeight: number;
  }>({ left: 0, width: 0, maxHeight: 0 });

  const close = () => {
    setOpen(false);
    triggerRef.current?.focus();
  };
  useEscape(open ? close : null);

  // Focus the current choice when the menu opens, that both scrolls a long
  // list to it and puts the arrow keys below on the right starting point.
  // Once, not on every re-render, which would fight the user's own scrolling.
  const listRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const list = listRef.current;
    if (!list) return;
    const target =
      list.querySelector<HTMLButtonElement>('[aria-selected="true"]:not([disabled])') ??
      list.querySelector<HTMLButtonElement>('[role="option"]:not([disabled])');
    // preventScroll + an explicit nearest scroll: a bare focus() can jerk the
    // whole page when the portal sits at the document root. Optional-call the
    // scroll, jsdom doesn't implement it, so tests would throw here.
    target?.focus({ preventScroll: true });
    target?.scrollIntoView?.({ block: "nearest" });
  }, [open]);

  // Measure on open (before paint, so the menu never flashes at 0,0).
  useLayoutEffect(() => {
    const el = triggerRef.current;
    if (!open || !el) return;
    const r = el.getBoundingClientRect();
    const below = window.innerHeight - r.bottom - GAP - MARGIN;
    const above = r.top - GAP - MARGIN;
    const openUp = below < 160 && above > below;
    setPos({
      left: r.left,
      width: r.width,
      maxHeight: Math.min(288, Math.max(120, openUp ? above : below)),
      ...(openUp ? { bottom: window.innerHeight - r.top + GAP } : { top: r.bottom + GAP }),
    });
  }, [open]);

  // The trigger's width is only a MINIMUM for the menu (see the style below), so
  // a narrow trigger, "Haiku 4.5" in a settings row, still shows full option
  // labels instead of "Opus 4.…". A menu that grew past the right edge would be
  // clipped, though, so once it has laid out at its content width, pull it back
  // inside the viewport. Second pass on purpose: the width isn't knowable until
  // the options have rendered. Guarded on an actual change, so it settles in one
  // extra frame rather than looping.
  useLayoutEffect(() => {
    const el = triggerRef.current;
    const list = listRef.current;
    if (!open || !el || !list) return;
    // Anchor off the LIVE trigger rect, not `pos.left`: both layout effects run
    // in the same commit, so on the opening pass `pos` is still its initial
    // zero, clamping against that pinned every menu to the left viewport edge.
    const desiredLeft = el.getBoundingClientRect().left;
    const width = list.getBoundingClientRect().width;
    const clamped = Math.max(MARGIN, Math.min(desiredLeft, window.innerWidth - width - MARGIN));
    if (Math.abs(clamped - pos.left) > 0.5) setPos((p) => ({ ...p, left: clamped }));
  }, [open, pos.left, options]);

  // Roving focus over the enabled options. Enter/Space need no handling, each
  // option is a real <button>, so activation is the browser's job.
  const moveFocus = (delta: number | "first" | "last") => {
    const list = listRef.current;
    if (!list) return;
    const items = Array.from(
      list.querySelectorAll<HTMLButtonElement>('[role="option"]:not([disabled])'),
    );
    if (items.length === 0) return;
    const from = items.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      delta === "first"
        ? 0
        : delta === "last"
          ? items.length - 1
          : Math.min(items.length - 1, Math.max(0, (from < 0 ? 0 : from) + delta));
    items[next]?.focus({ preventScroll: true });
    items[next]?.scrollIntoView?.({ block: "nearest" });
  };

  const selected = options.find((o) => o.value === value);
  const hasWidthClass = /(^|\s)(w-|min-w-|max-w-)/.test(className ?? "");

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        onClick={() => (open ? close() : setOpen(true))}
        onKeyDown={(e) => {
          if (!open && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
            e.preventDefault();
            setOpen(true);
          }
        }}
        className={cx(
          "flex items-center justify-between gap-2 rounded-md border border-edge bg-panel-2 px-2.5 py-1.5 text-left text-sm text-ink",
          // `w-full` only when the caller didn't ask for a width. Appending both
          // and hoping the caller's wins doesn't work: Tailwind sorts utilities
          // into the stylesheet itself, so `w-full` beats `w-56` no matter which
          // is written last, which is how the Title-model picker ended up 101px
          // wide with its options truncated to "Opus 4.…".
          !hasWidthClass && "w-full",
          "focus:border-ink-dim/70 focus:outline-none disabled:cursor-not-allowed disabled:opacity-50",
          className,
        )}
      >
        <span className={cx("min-w-0 truncate", !selected && placeholder && "text-ink-dim")}>
          {selected?.label ?? placeholder ?? value}
        </span>
        <ChevronDownIcon size={12} className="shrink-0 text-ink-dim" />
      </button>
      {open &&
        createPortal(
          <>
            {/* Backdrop closes the menu on any outside click. */}
            <div className="fixed inset-0 z-50" onClick={close} aria-hidden="true" />
            <div
              ref={listRef}
              role="listbox"
              aria-label={ariaLabel}
              onKeyDown={(e) => {
                const move =
                  e.key === "ArrowDown"
                    ? 1
                    : e.key === "ArrowUp"
                      ? -1
                      : e.key === "Home"
                        ? ("first" as const)
                        : e.key === "End"
                          ? ("last" as const)
                          : null;
                if (move === null) return;
                e.preventDefault();
                moveFocus(move);
              }}
              className="fixed z-50 overflow-y-auto rounded-md border border-edge bg-panel p-1 shadow-2xl"
              style={{
                top: pos.top,
                bottom: pos.bottom,
                left: pos.left,
                // Align with the trigger and never be narrower than it, but grow
                // to fit the longest label rather than truncating it, a native
                // select's popup sizes to its content too. Capped to the viewport
                // so a long model id can't run off-screen.
                minWidth: pos.width,
                maxWidth: `calc(100vw - ${MARGIN * 2}px)`,
                maxHeight: pos.maxHeight,
              }}
            >
              {options.map((o, i) => (
                <Fragment key={`${o.group ?? ""} ${o.value}`}>
                  {o.group && o.group !== options[i - 1]?.group && (
                    <div
                      role="presentation"
                      className="px-2.5 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wide text-ink-dim first:pt-1"
                    >
                      {o.group}
                    </div>
                  )}
                  <button
                    type="button"
                    role="option"
                    disabled={o.disabled}
                    aria-selected={o.value === value}
                    onClick={() => {
                      onChange(o.value);
                      close();
                    }}
                    className={cx(
                      "flex w-full items-center gap-2 rounded px-2.5 py-1.5 text-left text-sm",
                      "focus:outline-none enabled:hover:bg-panel-2 enabled:focus-visible:bg-panel-2",
                      o.disabled
                        ? "cursor-not-allowed text-ink-dim/50"
                        : o.value === value
                          ? "text-ink"
                          : "text-ink-dim hover:text-ink",
                    )}
                  >
                    <span className="min-w-0 flex-1 truncate">{o.label}</span>
                    {/* Decorative: `aria-selected` already says this is the
                        choice, and without hiding it the glyph lands in the
                        option's accessible name ("main✓"). */}
                    {o.value === value && (
                      <span aria-hidden="true" className="shrink-0 text-[11px]">
                        ✓
                      </span>
                    )}
                  </button>
                </Fragment>
              ))}
            </div>
          </>,
          document.body,
        )}
    </>
  );
}

/** One labelled settings row (title + description left, control right), shared
 *  by Settings, the Harnesses tabs and the agent editor. */
export function SettingRow({
  title,
  children,
  control,
}: {
  title: ReactNode;
  children?: ReactNode;
  control: ReactNode;
}) {
  return (
    <div className="flex items-start justify-between gap-4 py-3.5">
      <div className="min-w-0">
        <div className="flex items-center gap-2 text-sm text-ink">{title}</div>
        {children && <p className="mt-0.5 text-xs text-ink-dim">{children}</p>}
      </div>
      <div className="shrink-0 pt-0.5">{control}</div>
    </div>
  );
}

/** Labeled form row. NOTE: children render inside a `<label>`, and WebKit
 *  forwards clicks on any label descendant to the first labelable child,
 *  overlays/menus opened from inside a Field must portal out (see
 *  DropdownSelect) or every click on them re-triggers the control. */
export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="block text-sm">
      <span className="mb-1 block text-xs font-medium text-ink-dim">{label}</span>
      {children}
    </label>
  );
}

type ToggleProps = {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  /** Accessible label, since the switch renders no visible text itself. */
  "aria-label"?: string;
};

/** A binary on/off switch styled to match the settings rows. */
export function Toggle({ checked, onChange, disabled, ...rest }: ToggleProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={rest["aria-label"]}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cx(
        "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors",
        "focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60",
        "disabled:cursor-not-allowed disabled:opacity-50",
        checked ? "border-accent bg-accent" : "border-edge bg-panel-2",
      )}
    >
      <span
        className={cx(
          "inline-block h-3.5 w-3.5 transform rounded-full bg-surface transition-transform",
          checked ? "translate-x-4" : "translate-x-0.5",
        )}
      />
    </button>
  );
}

type DialogProps = {
  open: boolean;
  onClose: () => void;
  title?: ReactNode;
  children: ReactNode;
  className?: string;
  /** Names the dialog when the layout carries its own heading instead of `title`. */
  ariaLabel?: string;
  /** Drop the panel's inset so content can run edge-to-edge. */
  padded?: boolean;
  /** Let the panel scroll. Off when the content owns its own scroll regions. */
  scroll?: boolean;
};

/** Dependency-free modal: fixed overlay + panel; Escape or a backdrop click closes. */
export function Dialog({
  open,
  onClose,
  title,
  children,
  className,
  ariaLabel,
  padded = true,
  scroll = true,
}: DialogProps) {
  useEscape(open ? onClose : null);

  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-surface/70 p-4 backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={ariaLabel}
        className={cx(
          "max-h-full w-full max-w-md rounded-lg border border-edge bg-panel shadow-2xl",
          scroll ? "overflow-y-auto" : "overflow-hidden",
          padded && "p-4",
          className,
        )}
      >
        {title && <h2 className="mb-3 text-sm font-semibold tracking-wide text-ink">{title}</h2>}
        {children}
      </div>
    </div>
  );
}

export function ErrorNote({
  error,
  onDismiss,
}: {
  error: unknown;
  /** When provided, renders a × that clears the note. The caller owns the
   *  dismissal (e.g. resetting the mutation) so a fresh attempt re-shows it. */
  onDismiss?: () => void;
}) {
  // No error → render nothing. Without this a null/undefined error (e.g. a
  // mutation that hasn't run yet) paints a box reading "null", since
  // `errorMessage(null)` stringifies to "null".
  if (error == null) return null;
  const remedy = errorRemedy(error);
  return (
    <div className="selectable relative break-words rounded-md border border-bad/30 bg-bad/10 p-3 text-sm">
      {onDismiss && (
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss error"
          className="absolute right-1.5 top-1.5 flex h-5 w-5 items-center justify-center rounded text-bad/70 hover:bg-bad/15 hover:text-bad"
        >
          <XIcon size={11} />
        </button>
      )}
      <p className={cx("text-bad", onDismiss && "pr-5")}>{errorMessage(error)}</p>
      {remedy && <p className="mt-1 text-ink-dim">{remedy}</p>}
    </div>
  );
}

/** Standard route-level loading shell. */
export function PageSpinner() {
  return (
    <div className="flex justify-center p-12">
      <Spinner />
    </div>
  );
}

/** Standard route-level error shell. */
export function PageError({ error }: { error: unknown }) {
  return (
    <div className="p-8">
      <ErrorNote error={error} />
    </div>
  );
}

type IconButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  /** Highlighted "this panel is open" state. */
  active?: boolean;
};

/** Square h-8 icon-only button, the standard chrome control size. */
export function IconButton({ active, className, ...rest }: IconButtonProps) {
  return (
    <button
      type="button"
      className={cx(
        "inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md",
        "disabled:cursor-not-allowed disabled:opacity-50",
        active ? "bg-panel-2 text-ink" : "text-ink-dim hover:bg-panel-2 hover:text-ink",
        className,
      )}
      {...rest}
    />
  );
}

type RefreshButtonProps = {
  onClick: () => void;
  /** Swaps the icon for a spinner and blocks re-entry while a refetch is in flight. */
  busy?: boolean;
  /** Accessible name, say what gets refreshed. Wins over any visible `children`. */
  label: string;
  title?: string;
  className?: string;
  /** Visible text beside the icon, e.g. a sync-status line. Icon-only when omitted. */
  children?: ReactNode;
};

/** Borderless refresh control: an icon-only square, or icon + status text. */
export function RefreshButton({
  onClick,
  busy = false,
  label,
  title,
  className,
  children,
}: RefreshButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy}
      className={cx(
        "flex shrink-0 items-center gap-1.5 rounded text-ink-dim",
        "enabled:hover:text-ink disabled:cursor-default",
        children ? "px-1.5 py-1" : "h-6 w-6 justify-center enabled:hover:bg-edge/60",
        className,
      )}
      title={title ?? label}
      aria-label={label}
    >
      {busy ? <Spinner size={12} /> : <RestartIcon size={13} />}
      {children}
    </button>
  );
}

type ContextMenuProps = {
  position: { x: number; y: number };
  onClose: () => void;
  children: ReactNode;
};

/** The currently-open menu's closer, so opening a new one (even via right-click,
 *  which doesn't fire a `click`) dismisses any other, only one menu at a time. */
let closeActiveContextMenu: (() => void) | null = null;

/** Fixed-position right-click menu; any window click or Escape closes it, and
 *  opening another menu closes this one. Clamps itself into the viewport so a
 *  menu opened near a screen edge (e.g. a far-right overflow button) doesn't
 *  spill off-screen. */
export function ContextMenu({ position, onClose, children }: ContextMenuProps) {
  useEscape(onClose);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState(position);

  // Measure once mounted and pull the menu back inside the viewport (before
  // paint, so it never flashes off-screen). Re-runs when the anchor moves.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const margin = 8;
    setPos({
      x: Math.max(margin, Math.min(position.x, window.innerWidth - width - margin)),
      y: Math.max(margin, Math.min(position.y, window.innerHeight - height - margin)),
    });
  }, [position.x, position.y]);

  useEffect(() => {
    // Mounting (or repositioning) closes whatever other menu was open.
    if (closeActiveContextMenu && closeActiveContextMenu !== onClose) closeActiveContextMenu();
    closeActiveContextMenu = onClose;

    window.addEventListener("click", onClose);
    return () => {
      if (closeActiveContextMenu === onClose) closeActiveContextMenu = null;
      window.removeEventListener("click", onClose);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      role="menu"
      className="fixed z-50 min-w-44 rounded-md border border-edge bg-panel p-1 shadow-2xl"
      style={{ left: pos.x, top: pos.y }}
      onClick={(e) => e.stopPropagation()}
    >
      {children}
    </div>
  );
}

type ContextMenuItemProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  danger?: boolean;
  /** A second, muted line under the label, typically why the item is disabled
   *  (e.g. "Agent is still working, stop it first."). Rendered in warn tone. */
  hint?: ReactNode;
};

export function ContextMenuItem({ danger, hint, className, children, ...rest }: ContextMenuItemProps) {
  return (
    <button
      type="button"
      role="menuitem"
      className={cx(
        "block w-full rounded px-2.5 py-1.5 text-left text-sm",
        "disabled:cursor-not-allowed disabled:opacity-50",
        danger ? "text-bad hover:bg-bad/10" : "text-ink hover:bg-panel-2",
        className,
      )}
      {...rest}
    >
      {children}
      {hint && <span className="mt-0.5 block text-xs text-warn">{hint}</span>}
    </button>
  );
}
