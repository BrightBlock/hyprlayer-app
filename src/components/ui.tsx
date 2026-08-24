import { Fragment, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { twMerge } from "tailwind-merge";
import type {
  ButtonHTMLAttributes,
  HTMLAttributes,
  InputHTMLAttributes,
  ReactNode,
  RefObject,
} from "react";

import { errorMessage, errorRemedy } from "@/lib/ipc";
import { formatChord } from "@/lib/keybindings/chord";
import { useEscape } from "@/lib/keybindings/registry";
import { IS_MACOS } from "@/lib/platform";
import { ChevronDownIcon, RestartIcon, XIcon } from "./icons";

/** Join class fragments, letting later ones override earlier ones in the same
 *  utility group. A plain join can't: Tailwind sorts utilities into the
 *  stylesheet itself, so `max-w-md` (the Dialog base) beat every caller's
 *  `max-w-2xl` regardless of write order, and five dialogs shipped 448px wide.
 *  See the width-class note in DropdownSelect, which hand-rolled a workaround
 *  for the same defect before this. */
function cx(...parts: Array<string | false | null | undefined>): string {
  return twMerge(parts.filter(Boolean).join(" "));
}

/** The app's one keyboard-focus treatment. Mouse users never see it
 *  (`focus-visible`, not `focus`); keyboard users always do. Toggle carried
 *  this alone before the sweep — every other control relied on the UA
 *  default outline, which the dark surfaces render nearly invisibly. */
const FOCUS_RING = "focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60";

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
        FOCUS_RING,
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
        "placeholder:text-ink-dim/70 focus:border-ink-dim/70",
        FOCUS_RING,
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
  /** Overrides the trigger's text, leaving the menu's own labels alone. For a
   *  control that sits inline in a sentence, where the full option label is
   *  worth reading in the menu but too long for the line it anchors to.
   *  Suppresses `placeholder`'s dim styling — the override is real content,
   *  not an empty-state hint. */
  triggerLabel?: string;
  /** Observes the menu opening and closing without owning either. Lets a
   *  caller defer loading its options until the user actually reaches for
   *  the menu. */
  onOpenChange?: (open: boolean) => void;
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
  triggerLabel,
  onOpenChange,
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

  const openMenu = () => {
    setOpen(true);
    onOpenChange?.(true);
  };
  const close = () => {
    setOpen(false);
    onOpenChange?.(false);
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

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        onClick={() => (open ? close() : openMenu())}
        onKeyDown={(e) => {
          if (!open && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
            e.preventDefault();
            openMenu();
          }
        }}
        className={cx(
          "flex w-full items-center justify-between gap-2 rounded-md border border-edge bg-panel-2 px-2.5 py-1.5 text-left text-sm text-ink",
          "focus:border-ink-dim/70",
          FOCUS_RING,
          "disabled:cursor-not-allowed disabled:opacity-50",
          className,
        )}
      >
        <span
          className={cx(
            "min-w-0 truncate",
            !triggerLabel && !selected && placeholder && "text-ink-dim",
          )}
        >
          {triggerLabel ?? selected?.label ?? placeholder ?? value}
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
                // U+001F (unit separator) joins group and value into one React
                // key — it can't occur in either, the same guarantee a literal
                // NUL gave, without tripping ripgrep's binary-file detection on
                // every directory-wide search of this file.
                <Fragment key={`${o.group ?? ""}${o.value}`}>
                  {o.group && o.group !== options[i - 1]?.group && (
                    <div
                      role="presentation"
                      className="px-2.5 pb-1 pt-2 text-2xs font-semibold uppercase tracking-wide text-ink-dim first:pt-1"
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
                      <span aria-hidden="true" className="shrink-0 text-2xs">
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
        FOCUS_RING,
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

/** Dialog widths. Named so a caller can't hand `Dialog` a `max-w-*` that loses
 *  to its own base class — the defect that shipped five dialogs at 448px
 *  (`max-w-lg`/`max-w-2xl`/`max-w-3xl` callers, all silently clamped to the
 *  panel's own `max-w-md`, since `cx()` had no conflict resolution). */
const DIALOG_SIZE = {
  sm: "max-w-sm", // 384px — PromptArgsDialog
  md: "max-w-md", // 448px — the default, most confirms
  lg: "max-w-lg", // 512px — AgentBundleDialogs, the share dialogs
  xl: "max-w-xl", // 576px — model/provider pickers
  "2xl": "max-w-2xl", // 672px — the three CLI sign-in dialogs (h-80 terminal)
  "3xl": "max-w-3xl", // 768px — ProjectSettingsDialog
} as const;

type DialogProps = {
  open: boolean;
  onClose: () => void;
  title?: ReactNode;
  children: ReactNode;
  className?: string;
  /** Names the dialog when the layout carries its own heading instead of `title`. */
  ariaLabel?: string;
  /** Panel max-width. Never override this with a `max-w-*` className — see
   *  `DIALOG_SIZE`'s comment for why that used to silently lose. */
  size?: keyof typeof DIALOG_SIZE;
  /** Rendered in its own band below the body, outside the scroll region, so a
   *  long dialog's action buttons never scroll away with the content. */
  footer?: ReactNode;
  /** `top` pins the panel near the viewport top instead of centering it, for
   *  dialogs whose body scrolls a long list (the export/import prompt pickers). */
  align?: "center" | "top";
  /** `border-b` under the header and `border-t` above the footer, for a panel
   *  whose body runs full-bleed (`padded={false}`) and needs a visible seam
   *  against the inset header/footer content. */
  seams?: boolean;
  /** Drop the body band's inset so its content can run edge-to-edge. The
   *  header and footer bands keep their own padding regardless — dropping
   *  theirs too would leave a full-bleed dialog with no title/footer inset
   *  at all, and every caller that needs that layout already frames it in
   *  `children` (see DiscoverDialog). */
  padded?: boolean;
  /** Let the body band scroll. Off when the content owns its own scroll
   *  regions (DiscoverDialog's two-pane layout). */
  scroll?: boolean;
  /** Render a × beside the title. Opt-in, because most dialogs already end in a
   *  Cancel button and a second close control next to it reads as clutter. Turn
   *  it on for the ones that have no footer at all — otherwise Escape and a
   *  backdrop click are the only ways out, and neither is visible. */
  closable?: boolean;
};

/** Dependency-free modal: fixed overlay + panel; Escape or a backdrop click
 *  closes. The panel is a three-band flex column — header and footer are
 *  `shrink-0`, only the body scrolls — so a `footer` never scrolls away with
 *  a long body. */
export function Dialog({
  open,
  onClose,
  title,
  children,
  className,
  ariaLabel,
  size = "md",
  footer,
  align = "center",
  seams = false,
  padded = true,
  scroll = true,
  closable = false,
}: DialogProps) {
  useEscape(open ? onClose : null);

  if (!open) return null;
  return (
    <div
      className={cx(
        "fixed inset-0 z-50 flex justify-center bg-surface/70 p-4 backdrop-blur-sm",
        align === "top" ? "items-start pt-[6vh]" : "items-center",
      )}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={ariaLabel}
        className={cx(
          "flex max-h-full w-full flex-col overflow-hidden rounded-lg border border-edge bg-panel shadow-2xl",
          DIALOG_SIZE[size],
          className,
        )}
      >
        {(title || closable) && (
          <div
            className={cx(
              "flex shrink-0 items-start justify-between gap-2 px-4 py-3",
              seams && "border-b border-edge",
            )}
          >
            {title ? (
              <h2 className="min-w-0 text-sm font-semibold tracking-wide text-ink">{title}</h2>
            ) : (
              <span />
            )}
            {closable && (
              <button
                type="button"
                onClick={onClose}
                aria-label="Close dialog"
                className={cx(
                  "-mr-1 -mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink",
                  FOCUS_RING,
                )}
              >
                <XIcon size={13} />
              </button>
            )}
          </div>
        )}
        <div
          className={cx(
            "flex min-h-0 flex-1 flex-col",
            scroll && "overflow-y-auto",
            padded && "p-4",
          )}
        >
          {children}
        </div>
        {footer && (
          <div className={cx("shrink-0 px-4 py-3", seams && "border-t border-edge")}>{footer}</div>
        )}
      </div>
    </div>
  );
}

type ConfirmDialogProps = {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  confirmLabel: string;
  confirmVariant?: "primary" | "danger";
  onConfirm: () => void;
  busy?: boolean;
  error?: unknown;
  onDismissError?: () => void;
  /** Extra action between Cancel and Confirm, e.g. Archive's
   *  "Skip script & archive". Keeps the dismissive→committing order. */
  extraActions?: ReactNode;
  cancelLabel?: string;
  size?: keyof typeof DIALOG_SIZE;
  /** Focus the confirming button on open, so Enter commits. Opt-in: a confirm
   *  raised by the user's own click (Delete, Archive) should not pre-arm a
   *  destructive default, but one raised by the system in response to
   *  something else — `ConfirmQuitDialog`, off the backend's quit event —
   *  is answering a question the user already asked. */
  autoFocusConfirm?: boolean;
};

/** The app's one confirm-footer shape: dismissive action, then any extra
 *  action, then the committing one — never reconsider that order (every
 *  dialog footer in the app agrees on it). Covers the four hand-composed
 *  confirms that each reimplemented this footer before it existed.
 *  `RenameWorkstreamDialog` deliberately stays off this: it's a form with a
 *  `type="submit"` primary action, not a confirm. */
export function ConfirmDialog({
  open,
  onClose,
  title,
  children,
  confirmLabel,
  confirmVariant = "primary",
  onConfirm,
  busy = false,
  error,
  onDismissError,
  extraActions,
  cancelLabel = "Cancel",
  size,
  autoFocusConfirm = false,
}: ConfirmDialogProps) {
  return (
    <Dialog open={open} onClose={onClose} title={title} size={size}>
      <div className="space-y-3">
        {children}
        {error != null && <ErrorNote error={error} onDismiss={onDismissError} />}
        <div className="flex justify-end gap-2">
          <Button type="button" onClick={onClose} disabled={busy}>
            {cancelLabel}
          </Button>
          {extraActions}
          <Button
            type="button"
            variant={confirmVariant}
            busy={busy}
            autoFocus={autoFocusConfirm}
            onClick={onConfirm}
          >
            {confirmLabel}
          </Button>
        </div>
      </div>
    </Dialog>
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
          className={cx(
            "absolute right-1.5 top-1.5 flex h-6 w-6 items-center justify-center rounded text-bad/70 hover:bg-bad/15 hover:text-bad",
            FOCUS_RING,
          )}
        >
          <XIcon size={11} />
        </button>
      )}
      <p className={cx("text-bad", onDismiss && "pr-5")}>{errorMessage(error)}</p>
      {remedy && <p className="mt-1 text-ink-dim">{remedy}</p>}
    </div>
  );
}

/** Raw CLI output, for an outcome the app cannot summarise honestly. Trims
 *  and renders nothing when there is nothing to show, so callers can pass
 *  stdout/stderr unconditionally. Wraps long lines (`whitespace-pre-wrap
 *  break-words`): CLI text embeds full asset filenames and URLs that would
 *  otherwise force a horizontal scrollbar inside a short note. */
export function CliOutput({ text }: { text: string }) {
  const trimmed = text.trim();
  if (!trimmed) return null;
  return (
    <pre className="selectable mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md border border-edge bg-panel-2 p-2.5 font-mono text-xs text-ink-dim">
      {trimmed}
    </pre>
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

/** Square h-8 icon-only button, the standard chrome control size — for controls
 *  that sit in a toolbar or header. The other tier is the inline `h-6 w-6
 *  rounded` control for a control that sits *inside* another surface
 *  (`RefreshButton`'s icon branch, the Dialog and ErrorNote close buttons,
 *  `AgentList`'s row kebab): no exported component, just the class pair
 *  applied at each site. */
export function IconButton({ active, className, ...rest }: IconButtonProps) {
  return (
    <button
      type="button"
      className={cx(
        "inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md",
        FOCUS_RING,
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
        FOCUS_RING,
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
    setPos({
      x: Math.max(MARGIN, Math.min(position.x, window.innerWidth - width - MARGIN)),
      y: Math.max(MARGIN, Math.min(position.y, window.innerHeight - height - MARGIN)),
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
        FOCUS_RING,
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

type PopoverProps = {
  open: boolean;
  onClose: () => void;
  /** The trigger to anchor off. Measured on open, so overflow containers
   *  can't clip the panel. */
  anchorRef: RefObject<HTMLElement | null>;
  /** Preferred side; flips when there isn't room (DropdownSelect's rule). */
  side?: "bottom" | "top";
  align?: "start" | "end";
  /** Floors the panel's width at the anchor's rendered width rather than its
   *  own content width — for a menu meant to read as an extension of the
   *  control it opens from (`ProjectSwitcher`, `RunSplit`'s Configure menu),
   *  not a fixed action-menu width. */
  matchAnchorWidth?: boolean;
  /** Gap between the anchor and the panel, in px. Defaults to the same `GAP`
   *  the trigger-anchored menus already share. `RunSplit`'s Configure menu
   *  sets this to 0 so the panel reads as a continuation of the button
   *  group's own border, not a separate floating panel. */
  sideOffset?: number;
  className?: string;
  children: ReactNode;
};

/**
 * Anchored floating panel — a menu or content popover that opens off a
 * trigger, sibling to the point-anchored `ContextMenu` and the listbox
 * `DropdownSelect`. Carries four things already load-bearing in those two:
 * Escape via the shared registry (a popover inside a dialog closes itself
 * first, leaving the dialog open); a portal to `document.body` for the same
 * reason `DropdownSelect` needs one (WebKit forwards `<label>`-descendant
 * clicks to the labeled control, and a dialog's `backdrop-blur` makes
 * `position: fixed` behave relatively in WebKit); a measure-before-paint
 * layout effect so the panel never flashes at 0,0; and a clamp into the
 * viewport using the shared `MARGIN`.
 *
 * Ten hand-rolled floating panels predate this, nine of them with no Escape
 * handler at all and six on `z-30` — below a dialog's own overlay. This is
 * the fix for both: one primitive at `z-50`, on `useEscape`.
 */
export function Popover({
  open,
  onClose,
  anchorRef,
  side = "bottom",
  align = "start",
  matchAnchorWidth = false,
  sideOffset = GAP,
  className,
  children,
}: PopoverProps) {
  useEscape(open ? onClose : null);
  const panelRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{
    top?: number;
    bottom?: number;
    left: number;
    minWidth?: number;
  }>({ left: 0 });

  // Measure both the anchor and the panel's own natural size (before paint,
  // so it never flashes at 0,0), then pick a side and clamp horizontally.
  useLayoutEffect(() => {
    const anchor = anchorRef.current;
    const panel = panelRef.current;
    if (!open || !anchor || !panel) return;
    const a = anchor.getBoundingClientRect();
    const natural = panel.getBoundingClientRect();
    // `matchAnchorWidth` applies `minWidth: a.width` on the NEXT paint, so the
    // measured rect is the panel's natural width, which for a narrow menu is
    // smaller than what will actually render. Aligning `end` off that number
    // put the panel its own shortfall too far right on first open (ScriptDock's
    // Configure menu), and the viewport clamp underestimated the overhang by
    // the same amount. Position against the width the panel is about to have.
    const width = matchAnchorWidth ? Math.max(natural.width, a.width) : natural.width;
    const height = natural.height;

    const spaceBelow = window.innerHeight - a.bottom - sideOffset - MARGIN;
    const spaceAbove = a.top - sideOffset - MARGIN;
    const preferredFits = side === "bottom" ? spaceBelow >= height : spaceAbove >= height;
    const openBelow = preferredFits ? side === "bottom" : spaceBelow > spaceAbove;

    const left = align === "start" ? a.left : a.right - width;
    setPos({
      left: Math.max(MARGIN, Math.min(left, window.innerWidth - width - MARGIN)),
      minWidth: matchAnchorWidth ? a.width : undefined,
      ...(openBelow
        ? { top: a.bottom + sideOffset }
        : { bottom: window.innerHeight - a.top + sideOffset }),
    });
  }, [open, anchorRef, side, align, matchAnchorWidth, sideOffset]);

  if (!open) return null;
  return createPortal(
    <>
      {/* Backdrop closes the popover on any outside click. */}
      <div className="fixed inset-0 z-50" onClick={onClose} aria-hidden="true" />
      <div
        ref={panelRef}
        className={cx("fixed z-50 rounded-md border border-edge bg-panel p-1 shadow-2xl", className)}
        style={{ top: pos.top, bottom: pos.bottom, left: pos.left, minWidth: pos.minWidth }}
      >
        {children}
      </div>
    </>,
    document.body,
  );
}

/** Menu item. `ContextMenuItem` under a name that doesn't imply right-click,
 *  since the same item now serves dropdowns and popovers. Old name kept as an
 *  alias so the existing call sites don't churn. */
export const MenuItem = ContextMenuItem;

/** The one menu separator. Two idioms existed: `my-1 h-px bg-edge`
 *  (ProjectPage) and `my-1 border-t border-edge` (SessionChat, PromptLibrary).
 *  The h-px form wins: it can't inherit a stray border-color from a parent. */
export function MenuSeparator() {
  return <div className="my-1 h-px bg-edge" />;
}

/** The app's two row heights, each tied to the corner radius it already had
 *  before this existed: every 28px row (ChangesPanel, CommitsPanel) already
 *  used `rounded-md`, every 36px row (the sidebar tree, both palettes,
 *  AgentList's agent row) already used `rounded-lg`. Derived from what the
 *  tree does, not invented — there is deliberately no third tier: the Agents
 *  list's one-off 52px row held 20px of single-line content and belonged in
 *  `standard` all along. A row with no radius of its own (the command
 *  palettes, which clip to their panel's own `rounded-lg`) cancels it with
 *  `rounded-none` in `className`, the same conflict `cx()` resolves anywhere
 *  else. */
export type RowTier = "compact" | "standard"; // h-7 / h-9

export type RowSelection = "none" | "tint" | "spine";

const ROW_TIER: Record<RowTier, string> = {
  compact: "h-7 rounded-md",
  standard: "h-9 rounded-lg",
};

/** The list-row hover: a row tints toward the divider colour, distinct from
 *  `hover:bg-panel-2`'s "raise to the next surface" meaning on menu items and
 *  chrome buttons (`ContextMenuItem`, `IconButton`, `DropdownSelect`'s
 *  options). A caller with its own active/hover semantics — the command
 *  palettes' keyboard-highlighted row — overrides it in `className`, which
 *  wins the same way any other conflicting utility does. */
const ROW_HOVER = "cursor-pointer hover:bg-edge/50";

/**
 * The app's row vocabulary as a pure class-builder, for callers `Row` can't
 * cover: `Root.tsx`'s workstream row is a `<Link>`, `AgentList`'s checkbox
 * wrapper is a `<label>`, and several rows are plain `<button>`s. Layout
 * (`flex` vs the shared agent-list `grid`), horizontal padding, and indent
 * stay the caller's job — only height, radius, the hover/selection tint, and
 * the focus ring are shared.
 */
export function rowClass({
  tier,
  selected = "none",
  interactive = true,
  className,
}: {
  tier: RowTier;
  selected?: RowSelection;
  /** Adds the hover tint and pointer cursor. Off for a row whose own active
   *  state already supplies the highlight (Root.tsx's selected workstream
   *  row keeps its hover off while active, so hovering it can't flicker the
   *  tint back to the neutral row colour). */
  interactive?: boolean;
  className?: string;
}): string {
  return cx(
    "relative items-center",
    ROW_TIER[tier],
    interactive && ROW_HOVER,
    selected !== "none" && "bg-accent/15",
    FOCUS_RING,
    className,
  );
}

/** The 3px accent bar on a selected row. Existed verbatim at four sites in
 *  three files — Settings.tsx:1138, Root.tsx:1906, Root.tsx:2035,
 *  LibrarySidebar.tsx:146 — and was dropped by a fourth row (Root's pinned
 *  row) that is selectable in the same tree. Requires `relative` on the row,
 *  which `rowClass` always emits. */
export function RowSpine() {
  return <span className="absolute inset-y-0 left-0 w-[3px] bg-accent" aria-hidden="true" />;
}

type RowProps = HTMLAttributes<HTMLDivElement> & {
  tier: RowTier;
  selected?: RowSelection;
  interactive?: boolean;
};

/** `rowClass` in component form, for the common `<div>` row. The polymorphic
 *  callers above apply `rowClass` directly instead — an `as`-prop generic
 *  here would fight TypeScript for no benefit. */
export function Row({
  tier,
  selected = "none",
  interactive = true,
  className,
  children,
  ...rest
}: RowProps) {
  return (
    <div className={rowClass({ tier, selected, interactive, className })} {...rest}>
      {selected === "spine" && <RowSpine />}
      {children}
    </div>
  );
}
