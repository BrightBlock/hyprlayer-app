import { Fragment, useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { extendTailwindMerge } from "tailwind-merge";
import type {
  ButtonHTMLAttributes,
  HTMLAttributes,
  InputHTMLAttributes,
  ReactNode,
  Ref,
  RefObject,
  TextareaHTMLAttributes,
} from "react";

import type { StepStatus } from "@/lib/bindings";
import { errorMessage, errorRemedy } from "@/lib/ipc";
import { formatChord, type Chord } from "@/lib/keybindings/chord";
import { useEscape } from "@/lib/keybindings/registry";
import { IS_MACOS } from "@/lib/platform";
import { keyedSize, startEdgeDrag } from "@/lib/resize";
import { useFocusTrap } from "@/lib/useFocusTrap";
import { useRovingRadio } from "@/lib/useRovingRadio";
import { Icon, type IconName, type IconSize } from "./Icon";

// Unregistered names never conflict, so `rounded-card` would survive `rounded-md`, and `text-ui` would be read as a colour and dropped beside `text-ink`.
const twMerge = extendTailwindMerge({
  extend: {
    theme: {
      radius: ["card", "control"],
      text: ["display", "h1", "h2", "h3", "title", "body-lg", "body", "ui", "ui-sm", "meta", "label", "tag"],
      leading: ["prose"],
      shadow: ["card", "pop", "ring-active"],
    },
  },
});

export function cx(...parts: Array<string | false | null | undefined>): string {
  return twMerge(parts.filter(Boolean).join(" "));
}

// No opacity, snap on disable: WebKit leaves stale paint when an animating button shifts layout.
const PRESS =
  "transition-[scale] hover:duration-(--duration-instant) active:duration-(--duration-instant) motion-safe:enabled:active:scale-97 disabled:transition-none";

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

/** A shortcut shown as an editable field's value (keybinding and prompt editors). */
export function ChordPill({ chord, unsetLabel }: { chord: Chord | null; unsetLabel: string }) {
  if (!chord) return <span className="text-ui italic text-ink-dim">{unsetLabel}</span>;
  return (
    <kbd
      className="inline-flex items-center rounded-md border border-edge bg-panel-2 px-2.5 py-1.5 text-body font-medium text-ink"
      style={IS_MACOS ? { letterSpacing: "0.1em", paddingRight: "calc(0.625rem - 0.1em)" } : undefined}
    >
      {formatChord(chord, IS_MACOS)}
    </kbd>
  );
}

const KBD_CHIP = "inline-flex items-center rounded-control border border-edge bg-panel/70 px-1.5 py-0.5 text-meta text-ink-dim";

export function Kbd({ chord, children, className }: { chord?: Chord | null; children?: ReactNode; className?: string }) {
  if (chord === null) return null;
  return chord ? (
    <ChordHint chord={chord} className={cx(KBD_CHIP, className)} />
  ) : (
    <kbd className={cx(KBD_CHIP, className)}>{children}</kbd>
  );
}

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "ghost" | "danger";
  size?: "sm" | "md";
  busy?: boolean;
  ref?: Ref<HTMLButtonElement>;
};

export function Button({
  variant = "ghost",
  size = "md",
  busy,
  className,
  children,
  disabled,
  ...rest
}: ButtonProps) {
  return (
    <button
      className={cx(
        "inline-flex items-center gap-2 rounded-control px-3 py-1.5 text-body font-medium",
        PRESS,
        size === "sm" && "h-6 gap-1.5 px-2 py-0 text-ui",
        "disabled:cursor-not-allowed disabled:opacity-50",
        variant === "primary" &&
          "border border-accent bg-accent text-accent-ink hover:bg-accent-hover focus-visible:bg-accent-hover",
        variant === "ghost" && "border border-edge bg-panel-2 hover:bg-edge/60 focus-visible:bg-edge/60",
        variant === "danger" && "border border-bad/25 text-bad-ink hover:bg-bad/10 focus-visible:bg-bad/10",
        className,
      )}
      disabled={disabled || busy}
      {...rest}
    >
      {busy && <Spinner size={size === "sm" ? "xs" : "sm"} />}
      {children}
    </button>
  );
}

export function Card({ className, ...rest }: HTMLAttributes<HTMLDivElement>) {
  return <div className={cx("rounded-card border border-edge bg-panel p-4", className)} {...rest} />;
}

export function CardTitle({ className, ...rest }: HTMLAttributes<HTMLHeadingElement>) {
  return <h2 className={cx("mb-3 text-body font-semibold text-ink", className)} {...rest} />;
}

export type BadgeTone = "ok" | "warn" | "bad" | "dim" | "accent";

export function Badge({ tone = "dim", children }: { tone?: BadgeTone; children: ReactNode }) {
  const tones: Record<BadgeTone, string> = {
    ok: "bg-ok/15 text-ok",
    warn: "bg-warn/15 text-warn-ink",
    bad: "bg-bad/15 text-bad",
    dim: "bg-edge/60 text-ink-dim",
    accent: "border border-edge bg-panel-2 text-ink",
  };
  return (
    <span className={cx("inline-flex items-center rounded-control px-1.5 py-0.5 text-ui font-medium", tones[tone])}>
      {children}
    </span>
  );
}

export function Spinner({ size = "md" }: { size?: IconSize }) {
  return <Icon name="loading" size={size} label="Loading" className="animate-spin text-ink-dim" />;
}

/** A placeholder bar; static while waiting, shimmering while it loads. */
export function Skeleton({ className, shimmer }: { className?: string; shimmer?: boolean }) {
  return (
    <span
      aria-hidden
      className={cx("block rounded-control bg-edge/70", shimmer ? "animate-shimmer" : "opacity-75", className)}
    />
  );
}

export function EmptyState({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 p-12 text-center">
      <p className="text-title text-ink">{title}</p>
      {children && <div className="max-w-md text-body text-ink-dim">{children}</div>}
    </div>
  );
}

export function TextInput({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      className={cx(
        "selectable w-full rounded-control border border-edge bg-panel-2 px-2.5 py-1.5 text-body text-ink",
        "placeholder:text-ink-dim/70 focus:border-ink-dim/70",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...rest}
    />
  );
}

const TEXTAREA_MAX_LINES = 8;

// Controlled only: it re-measures when `value` changes.
export function TextArea({
  className,
  value,
  rows = 2,
  ref,
  ...rest
}: TextareaHTMLAttributes<HTMLTextAreaElement> & { ref?: Ref<HTMLTextAreaElement> }) {
  const inner = useRef<HTMLTextAreaElement | null>(null);
  const setRef = useCallback(
    (el: HTMLTextAreaElement | null) => {
      inner.current = el;
      if (typeof ref === "function") ref(el);
      else if (ref) ref.current = el;
    },
    [ref],
  );

  useLayoutEffect(() => {
    const el = inner.current;
    // A portal into a not-yet-attached container (CodeMirror widgets) measures as zero height.
    if (!el?.isConnected) return;
    const cs = getComputedStyle(el);
    const px = (v: string) => parseFloat(v) || 0;
    const border = px(cs.borderTopWidth) + px(cs.borderBottomWidth);
    const max = px(cs.lineHeight) * TEXTAREA_MAX_LINES + px(cs.paddingTop) + px(cs.paddingBottom) + border;
    el.style.height = "auto";
    const natural = el.scrollHeight + border;
    el.style.height = `${Math.min(natural, max)}px`;
    el.style.overflowY = natural > max ? "auto" : "hidden";
  }, [value]);

  return (
    <textarea
      ref={setRef}
      value={value}
      rows={rows}
      className={cx(
        "selectable block w-full resize-none rounded-control border border-edge bg-panel-2 px-2.5 py-1.5 text-body text-ink",
        "placeholder:text-ink-dim/70 focus:border-ink-dim/70",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...rest}
    />
  );
}

const GAP = 4;
const MARGIN = 8;

export type DropdownOption = {
  value: string;
  label: string;
  disabled?: boolean;
  group?: string;
};

type DropdownSelectProps = {
  value: string;
  options: DropdownOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
  className?: string;
  "aria-label"?: string;
  placeholder?: string;
  triggerLabel?: string;
  onOpenChange?: (open: boolean) => void;
};

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

  const listRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const list = listRef.current;
    if (!list) return;
    const target =
      list.querySelector<HTMLButtonElement>('[aria-selected="true"]:not([disabled])') ??
      list.querySelector<HTMLButtonElement>('[role="option"]:not([disabled])');
    target?.focus({ preventScroll: true });
    target?.scrollIntoView?.({ block: "nearest" });
  }, [open]);

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

  useLayoutEffect(() => {
    const el = triggerRef.current;
    const list = listRef.current;
    if (!open || !el || !list) return;
    const desiredLeft = el.getBoundingClientRect().left;
    // Layout width: the rect is scaled down while `animate-pop-in` runs.
    const width = list.offsetWidth;
    const clamped = Math.max(MARGIN, Math.min(desiredLeft, window.innerWidth - width - MARGIN));
    if (Math.abs(clamped - pos.left) > 0.5) setPos((p) => ({ ...p, left: clamped }));
  }, [open, pos.left, options]);

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
          "flex w-full items-center justify-between gap-2 rounded-control border border-edge bg-panel-2 px-2.5 py-1.5 text-left text-body text-ink",
          "focus:border-ink-dim/70",
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
        <Icon name="chevronDown" size="xs" className="text-ink-dim" />
      </button>
      {open &&
        createPortal(
          <>
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
              className={cx(
                "fixed z-50 overflow-y-auto rounded-card border border-edge bg-panel p-1 shadow-pop animate-pop-in",
                pos.bottom === undefined ? "origin-top-left" : "origin-bottom-left",
              )}
              style={{
                top: pos.top,
                bottom: pos.bottom,
                left: pos.left,
                minWidth: pos.width,
                maxWidth: `calc(100vw - ${MARGIN * 2}px)`,
                maxHeight: pos.maxHeight,
              }}
            >
              {options.map((o, i) => (
                <Fragment key={`${o.group ?? ""}${o.value}`}>
                  {o.group && o.group !== options[i - 1]?.group && (
                    <div
                      role="presentation"
                      className="px-2.5 pb-1 pt-2 text-label text-ink-dim first:pt-1"
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
                      "flex w-full items-center gap-2 rounded-control px-2.5 py-1.5 text-left text-body",
                      "focus:outline-none enabled:hover:bg-panel-2 enabled:focus-visible:bg-panel-2",
                      o.disabled
                        ? "cursor-not-allowed text-ink-dim/50"
                        : o.value === value
                          ? "text-ink"
                          : "text-ink-dim hover:text-ink",
                    )}
                  >
                    <span className="min-w-0 flex-1 truncate">{o.label}</span>
                    {o.value === value && (
                      <Icon name="check" size="sm" />
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
        <div className="flex items-center gap-2 text-body text-ink">{title}</div>
        {children && <p className="mt-0.5 text-ui text-ink-dim">{children}</p>}
      </div>
      <div className="shrink-0 pt-0.5">{control}</div>
    </div>
  );
}

export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="block text-body">
      <span className="mb-1 block text-ui font-medium text-ink-dim">{label}</span>
      {children}
    </label>
  );
}

type ToggleProps = {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  className?: string;
  "aria-describedby"?: string;
} & (
  | { "aria-label": string; children?: undefined }
  | { children: ReactNode; "aria-label"?: string }
);

export function Toggle({
  checked,
  onChange,
  disabled,
  className,
  children,
  "aria-label": ariaLabel,
  "aria-describedby": describedBy,
}: ToggleProps) {
  return (
    <label
      className={cx(
        "relative inline-flex shrink-0 items-center gap-2",
        disabled ? "cursor-not-allowed opacity-50" : "cursor-pointer",
        className,
      )}
    >
      {children}
      <input
        type="checkbox"
        role="switch"
        className="peer sr-only"
        checked={checked}
        disabled={disabled}
        aria-label={ariaLabel}
        aria-describedby={describedBy}
        onChange={(e) => {
          if (!disabled) onChange(e.currentTarget.checked);
        }}
      />
      <span
        aria-hidden="true"
        className={cx(
          "relative h-[18px] w-[34px] shrink-0 rounded-full",
          "peer-focus-visible:ring-2 peer-focus-visible:ring-accent/60",
          checked ? "bg-accent" : "bg-track",
        )}
      >
        <span
          className={cx(
            "absolute left-[3px] top-[3px] h-3 w-3 rounded-full bg-accent-ink transition-transform",
            checked && "translate-x-4",
          )}
        />
      </span>
    </label>
  );
}

type SegmentedControlProps<T extends string> = {
  value: T | null;
  options: ReadonlyArray<{ value: T; label: ReactNode; disabled?: boolean; title?: string }>;
  onChange: (value: T) => void;
  disabled?: boolean;
  className?: string;
  "aria-label": string;
  /** `bordered`: a toolbar toggle that sits on a panel (the Library's width control). */
  variant?: "default" | "bordered";
};

export function SegmentedControl<T extends string>({
  value,
  options,
  onChange,
  disabled,
  className,
  "aria-label": ariaLabel,
  variant = "default",
}: SegmentedControlProps<T>) {
  const { groupProps, itemProps } = useRovingRadio(options, value, onChange, disabled);
  const bordered = variant === "bordered";
  return (
    <div
      {...groupProps}
      aria-label={ariaLabel}
      className={cx(
        "inline-flex rounded-control bg-panel-2 p-0.5",
        bordered && "items-center gap-0.5 border border-edge",
        className,
      )}
    >
      {options.map((o, i) => (
        <button
          key={o.value}
          type="button"
          {...itemProps(i)}
          title={o.title}
          className={cx(
            "rounded-[2px] text-ui",
            bordered ? "flex h-5 items-center px-2" : "px-2.5 py-0.5",
            "disabled:cursor-not-allowed disabled:opacity-50",
            o.value === value
              ? bordered
                ? "bg-accent-soft font-semibold text-ink ring-1 ring-inset ring-edge"
                : "bg-panel font-semibold text-ink shadow-sm"
              : bordered
                ? "font-medium text-ink-dim enabled:hover:text-ink"
                : "text-ink-2 enabled:hover:text-ink",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

const DIALOG_SIZE = {
  sm: "max-w-sm",
  md: "max-w-md",
  lg: "max-w-lg",
  xl: "max-w-xl",
  "2xl": "max-w-2xl",
  "3xl": "max-w-3xl",
  "4xl": "max-w-4xl",
} as const;

type DialogProps = {
  open: boolean;
  onClose: () => void;
  title?: ReactNode;
  children: ReactNode;
  className?: string;
  ariaLabel?: string;
  size?: keyof typeof DIALOG_SIZE;
  footer?: ReactNode;
  align?: "center" | "top";
  seams?: boolean;
  padded?: boolean;
  scroll?: boolean;
  closable?: boolean;
};

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
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement>(null);
  // Read during render: an autoFocus child takes focus at commit, before any effect could see the opener.
  const openerRef = useRef<HTMLElement | null>(null);
  const wasOpen = useRef(false);
  if (open && !wasOpen.current) {
    openerRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  }
  wasOpen.current = open;
  const onTrapKeyDown = useFocusTrap(panelRef, open, { restoreFocusRef: openerRef });

  if (!open) return null;
  return (
    <div
      className={cx(
        "fixed inset-0 z-50 flex animate-fade-in justify-center bg-surface/70 p-4 backdrop-blur-sm",
        align === "top" ? "items-start pt-[6vh]" : "items-center",
      )}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={ariaLabel}
        aria-labelledby={!ariaLabel && title ? titleId : undefined}
        tabIndex={-1}
        onKeyDown={onTrapKeyDown}
        className={cx(
          "flex max-h-full w-full animate-dialog-in outline-none flex-col overflow-hidden rounded-card border border-edge bg-panel shadow-2xl",
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
              <h2 id={titleId} className="min-w-0 text-body font-semibold text-ink">
                {title}
              </h2>
            ) : (
              <span />
            )}
            {closable && (
              <button
                type="button"
                onClick={onClose}
                aria-label="Close dialog"
                className={cx(
                  "-mr-1 -mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-control text-ink-dim hover:bg-edge/60 hover:text-ink",
                  "focus-visible:bg-edge/60 focus-visible:text-ink",
                )}
              >
                <Icon name="close" size="sm" />
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
  extraActions?: ReactNode;
  cancelLabel?: string;
  size?: keyof typeof DIALOG_SIZE;
  /** Which button takes focus on open, so Enter acts on it. */
  autoFocus?: "confirm" | "cancel";
};

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
  autoFocus,
}: ConfirmDialogProps) {
  return (
    <Dialog open={open} onClose={onClose} title={title} size={size} ariaLabel={title}>
      <div className="space-y-3">
        {children}
        {error != null && <ErrorNote error={error} onDismiss={onDismissError} />}
        <div className="flex justify-end gap-2">
          <Button type="button" onClick={onClose} disabled={busy} autoFocus={autoFocus === "cancel"}>
            {cancelLabel}
          </Button>
          {extraActions}
          <Button
            type="button"
            variant={confirmVariant}
            busy={busy}
            autoFocus={autoFocus === "confirm"}
            onClick={onConfirm}
          >
            {confirmLabel}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

type ConfirmRequest = {
  title: string;
  message?: string;
  confirmLabel: string;
  confirmVariant?: "primary" | "danger";
};

/** `confirm()` in-app: render `dialog`, await `confirm`. Only the confirm button answers true. */
export function useConfirm(): [dialog: ReactNode, confirm: (request: ConfirmRequest) => Promise<boolean>] {
  const [request, setRequest] = useState<ConfirmRequest | null>(null);
  const settle = useRef<((ok: boolean) => void) | null>(null);
  const opener = useRef<HTMLElement | null>(null);

  const confirm = useCallback((next: ConfirmRequest) => {
    if (settle.current) settle.current(false);
    else opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setRequest(next);
    return new Promise<boolean>((resolve) => {
      settle.current = resolve;
    });
  }, []);

  const answer = (ok: boolean) => {
    settle.current?.(ok);
    settle.current = null;
    setRequest(null);
  };

  useEffect(() => () => settle.current?.(false), []);

  // As `useFocusTrap`: focus goes back to a keyboard user only.
  const open = request !== null;
  useEffect(() => {
    if (!open) return;
    let byPointer = false;
    const onPointer = () => {
      byPointer = true;
    };
    const onKey = () => {
      byPointer = false;
    };
    document.addEventListener("pointerdown", onPointer, true);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("pointerdown", onPointer, true);
      document.removeEventListener("keydown", onKey, true);
      if (!byPointer && opener.current?.isConnected) opener.current.focus({ preventScroll: true });
    };
  }, [open]);

  const dialog = (
    <ConfirmDialog
      open={open}
      onClose={() => answer(false)}
      title={request?.title ?? ""}
      confirmLabel={request?.confirmLabel ?? ""}
      confirmVariant={request?.confirmVariant}
      autoFocus="confirm"
      onConfirm={() => answer(true)}
    >
      {request?.message && <p className="text-body leading-prose text-ink-dim">{request.message}</p>}
    </ConfirmDialog>
  );
  return [dialog, confirm];
}

export function ErrorNote({
  error,
  onDismiss,
}: {
  error: unknown;
  onDismiss?: () => void;
}) {
  if (error == null) return null;
  const remedy = errorRemedy(error);
  return (
    <div className="selectable relative break-words rounded-control border border-bad/30 bg-bad/10 p-3 text-body">
      {onDismiss && (
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss error"
          className={cx(
            "absolute right-1.5 top-1.5 flex h-6 w-6 items-center justify-center rounded-control text-bad/70 hover:bg-bad/15 hover:text-bad",
            "focus-visible:bg-bad/15 focus-visible:text-bad",
          )}
        >
          <Icon name="close" size="xs" />
        </button>
      )}
      <p className={cx("text-bad", onDismiss && "pr-5")}>{errorMessage(error)}</p>
      {remedy && <p className="mt-1 text-ink-dim">{remedy}</p>}
    </div>
  );
}

export function SaveStatus({
  saving,
  dirty,
  justSaved,
  readOnly,
}: {
  saving: boolean;
  dirty: boolean;
  justSaved: boolean;
  readOnly: boolean;
}) {
  if (readOnly) return <span className="text-ui-sm text-ink-dim">read-only</span>;
  if (saving) {
    return (
      <span className="flex items-center gap-1 text-ui-sm text-ink-dim">
        <Spinner size="xs" />
        Saving…
      </span>
    );
  }
  if (dirty) {
    return (
      <span className="flex items-center gap-1 text-ui-sm text-warn-ink">
        <span className="h-1.5 w-1.5 rounded-full bg-warn" aria-hidden="true" />
        Unsaved
      </span>
    );
  }
  if (justSaved) {
    return (
      <span className="flex items-center gap-1 text-ui-sm text-ok">
        <Icon name="check" size="xs" />
        Saved
      </span>
    );
  }
  return null;
}

export function CliOutput({ text }: { text: string }) {
  const trimmed = text.trim();
  if (!trimmed) return null;
  return (
    <pre className="selectable mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-control border border-edge bg-panel-2 p-2.5 text-meta text-ink-dim">
      {trimmed}
    </pre>
  );
}

export function PageSpinner() {
  return (
    <div className="flex justify-center p-12">
      <Spinner />
    </div>
  );
}

export function PageError({ error }: { error: unknown }) {
  return (
    <div className="p-8">
      <ErrorNote error={error} />
    </div>
  );
}

type IconButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  active?: boolean;
  ref?: Ref<HTMLButtonElement>;
};

export function IconButton({ active, className, ...rest }: IconButtonProps) {
  return (
    <button
      type="button"
      className={cx(
        "inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-control",
        PRESS,
        "disabled:cursor-not-allowed disabled:opacity-50",
        active
          ? "bg-panel-2 text-ink"
          : "text-ink-dim hover:bg-panel-2 hover:text-ink focus-visible:bg-panel-2 focus-visible:text-ink",
        className,
      )}
      {...rest}
    />
  );
}

// Rail tier (`Root.tsx`'s icon rail): `h-8 w-8 rounded-[10px]`. Selected state
// is a left `::before` bar, not a background fill — see ADR-0006.
export const RAIL_ICON_BASE =
  "relative flex h-8 w-8 items-center justify-center rounded-[10px]";
const RAIL_ACTIVE_BAR =
  "before:absolute before:-left-2 before:top-1/2 before:h-8 before:w-[2px] " +
  "before:-translate-y-1/2 before:rounded-r-full before:bg-accent before:content-['']";
export const railNavClass = (active: boolean): string =>
  `${RAIL_ICON_BASE} ${
    active ? `text-ink ${RAIL_ACTIVE_BAR}` : "text-ink-dim hover:bg-edge/60 hover:text-ink"
  }`;

type RefreshButtonProps = {
  onClick: () => void;
  busy?: boolean;
  label: string;
  title?: string;
  className?: string;
  children?: ReactNode;
};

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
        "flex shrink-0 items-center gap-1.5 rounded-control text-ink-dim",
        "enabled:hover:text-ink enabled:focus-visible:text-ink disabled:cursor-default",
        children ? "px-1.5 py-1" : "h-6 w-6 justify-center enabled:hover:bg-edge/60 enabled:focus-visible:bg-edge/60",
        className,
      )}
      title={title ?? label}
      aria-label={label}
    >
      {busy ? <Spinner size="sm" /> : <Icon name="sync" size="sm" />}
      {children}
    </button>
  );
}

type ContextMenuProps = {
  position: { x: number; y: number };
  onClose: () => void;
  children: ReactNode;
};

let closeActiveContextMenu: (() => void) | null = null;

export function ContextMenu({ position, onClose, children }: ContextMenuProps) {
  useEscape(onClose);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState(position);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const width = el.offsetWidth;
    const height = el.offsetHeight;
    setPos({
      x: Math.max(MARGIN, Math.min(position.x, window.innerWidth - width - MARGIN)),
      y: Math.max(MARGIN, Math.min(position.y, window.innerHeight - height - MARGIN)),
    });
  }, [position.x, position.y]);

  useEffect(() => {
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
      className="fixed z-50 min-w-44 origin-top-left animate-pop-in rounded-card border border-edge bg-panel p-1 shadow-pop"
      style={{ left: pos.x, top: pos.y }}
      onClick={(e) => e.stopPropagation()}
    >
      {children}
    </div>
  );
}

type ContextMenuItemProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  danger?: boolean;
  hint?: ReactNode;
  icon?: IconName;
  chord?: Chord | null;
};

export function ContextMenuItem({ danger, hint, icon, chord, className, children, ...rest }: ContextMenuItemProps) {
  return (
    <button
      type="button"
      role="menuitem"
      className={cx(
        "block w-full rounded-control px-2.5 py-1.5 text-left text-body",
        "disabled:cursor-not-allowed disabled:opacity-50",
        danger ? "text-bad hover:bg-bad/10 focus-visible:bg-bad/10" : "text-ink hover:bg-panel-2 focus-visible:bg-panel-2",
        className,
      )}
      {...rest}
    >
      {icon || chord ? (
        <span className="flex items-center gap-2">
          {icon && <Icon name={icon} size="sm" className={danger ? undefined : "text-ink-dim"} />}
          <span className="min-w-0 flex-1">{children}</span>
          {chord && <ChordHint chord={chord} className="ml-4 text-meta text-ink-dim" />}
        </span>
      ) : (
        children
      )}
      {hint && <span className="mt-0.5 block text-ui text-warn-ink">{hint}</span>}
    </button>
  );
}

type PopoverProps = {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLElement | null>;
  side?: "bottom" | "top";
  align?: "start" | "end";
  matchAnchorWidth?: boolean;
  sideOffset?: number;
  className?: string;
  children: ReactNode;
  caret?: boolean;
  /** Makes the panel a modal dialog: focus moves in, Tab cycles, focus returns on close. */
  trapFocus?: boolean;
  initialFocusRef?: RefObject<HTMLElement | null>;
  /** Defaults to the anchor. */
  restoreFocusRef?: RefObject<HTMLElement | null>;
  "aria-label"?: string;
  "aria-labelledby"?: string;
  /** False makes a hover card: no click-away backdrop, and Escape stays with whoever had it. */
  modal?: boolean;
};

const POP_ORIGIN = {
  below: { start: "origin-top-left", end: "origin-top-right" },
  above: { start: "origin-bottom-left", end: "origin-bottom-right" },
} as const;

const CARET = 9;
const CARET_INSET = 8;

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
  caret = false,
  trapFocus = false,
  initialFocusRef,
  restoreFocusRef,
  "aria-label": ariaLabel,
  "aria-labelledby": ariaLabelledBy,
  modal = true,
}: PopoverProps) {
  useEscape(open && modal ? onClose : null);
  const panelRef = useRef<HTMLDivElement>(null);
  const onTrapKeyDown = useFocusTrap(panelRef, open && trapFocus, {
    initialFocusRef,
    restoreFocusRef: restoreFocusRef ?? anchorRef,
  });
  const [pos, setPos] = useState<{
    top?: number;
    bottom?: number;
    left: number;
    minWidth?: number;
    caretLeft?: number;
    below?: boolean;
  }>({ left: 0 });

  useLayoutEffect(() => {
    if (!open) return;
    // Re-placed on resize, which app zoom also fires.
    const place = () => {
      const anchor = anchorRef.current;
      const panel = panelRef.current;
      if (!anchor || !panel) return;
      const a = anchor.getBoundingClientRect();
      // Layout size: the rect is scaled down while `animate-pop-in` runs.
      const width = matchAnchorWidth ? Math.max(panel.offsetWidth, a.width) : panel.offsetWidth;
      const height = panel.offsetHeight;

      const spaceBelow = window.innerHeight - a.bottom - sideOffset - MARGIN;
      const spaceAbove = a.top - sideOffset - MARGIN;
      const preferredFits = side === "bottom" ? spaceBelow >= height : spaceAbove >= height;
      const openBelow = preferredFits ? side === "bottom" : spaceBelow > spaceAbove;

      const desired = align === "start" ? a.left : a.right - width;
      const left = Math.max(MARGIN, Math.min(desired, window.innerWidth - width - MARGIN));
      const caretLeft = Math.max(
        CARET_INSET,
        Math.min(a.left + a.width / 2 - left - CARET / 2, width - CARET_INSET - CARET),
      );
      setPos({
        left,
        minWidth: matchAnchorWidth ? a.width : undefined,
        caretLeft,
        below: openBelow,
        ...(openBelow
          ? { top: a.bottom + sideOffset }
          : { bottom: window.innerHeight - a.top + sideOffset }),
      });
    };
    place();
    window.addEventListener("resize", place);
    return () => window.removeEventListener("resize", place);
  }, [open, anchorRef, side, align, matchAnchorWidth, sideOffset]);

  if (!open) return null;
  return createPortal(
    <>
      {modal && <div className="fixed inset-0 z-50" onClick={onClose} aria-hidden="true" />}
      <div
        ref={panelRef}
        role={trapFocus ? "dialog" : undefined}
        aria-modal={trapFocus ? true : undefined}
        aria-label={ariaLabel}
        aria-labelledby={ariaLabelledBy}
        tabIndex={trapFocus ? -1 : undefined}
        onKeyDown={onTrapKeyDown}
        className={cx(
          "fixed z-50 rounded-card border border-edge bg-panel p-1 shadow-pop outline-none animate-pop-in",
          POP_ORIGIN[pos.below ? "below" : "above"][align],
          className,
        )}
        style={{ top: pos.top, bottom: pos.bottom, left: pos.left, minWidth: pos.minWidth }}
      >
        {caret && (
          // Below the content but above the panel's border, which it covers where they meet.
          <span
            aria-hidden="true"
            className={cx(
              "absolute -z-10 h-[9px] w-[9px] rotate-45 border-edge bg-panel",
              pos.below ? "-top-[5px] border-l border-t" : "-bottom-[5px] border-b border-r",
            )}
            style={{ left: pos.caretLeft }}
          />
        )}
        {children}
      </div>
    </>,
    document.body,
  );
}

export const MenuItem = ContextMenuItem;

/** The status bar's popovers (Settings, Servers): the panel and its link rows. */
export const BAR_POPOVER = "w-[306px] p-1.5 shadow-pop";
export const BAR_POPOVER_ROW =
  "flex items-center gap-2.5 rounded-control px-3 py-2 text-ui text-ink-2 hover:bg-edge/50 " +
  "focus:outline-none focus-visible:bg-edge/50";

/** Archive popovers' row buttons (Restore, Delete, Keep). */
export const ROW_ACTION = cx(
  "inline-flex h-6 shrink-0 items-center rounded-control border border-edge bg-panel px-2",
  "text-ui-sm font-semibold text-ink-2 hover:bg-edge/50 disabled:opacity-50 disabled:hover:bg-panel",
  "focus:outline-none focus-visible:bg-edge/50",
);
export const ROW_ACTION_DANGER = cx(
  ROW_ACTION,
  "border-destructive-border text-bad-ink hover:bg-bad/10 focus-visible:bg-bad/10",
);

export function MenuSeparator() {
  return <div className="my-1 h-px bg-edge" />;
}

type RowTier = "compact" | "standard";

type RowSelection = "none" | "tint" | "spine";

const ROW_TIER: Record<RowTier, string> = {
  compact: "h-7 rounded-md",
  standard: "h-9 rounded-lg",
};

const ROW_HOVER = "cursor-pointer hover:bg-edge/50";

export function rowClass({
  tier,
  selected = "none",
  interactive = true,
  className,
}: {
  tier: RowTier;
  selected?: RowSelection;
  interactive?: boolean;
  className?: string;
}): string {
  return cx(
    "relative items-center",
    ROW_TIER[tier],
    interactive && ROW_HOVER,
    selected !== "none" && "bg-accent-soft",
    // A focus fill would outrank the selection tint.
    interactive && selected === "none" && "focus-visible:bg-edge/50",
    className,
  );
}

export function RowSpine() {
  return <span className="absolute inset-y-0 left-0 w-[3px] bg-accent" aria-hidden="true" />;
}

export function PaneSeparator({
  label,
  size,
  min,
  max,
  axis = "x",
  direction = 1,
  className,
  onResize,
  onEnd,
  "data-tour": tour,
}: {
  label: string;
  size: number;
  min: number;
  max: number;
  axis?: "x" | "y";
  // 1 when the panel grows as the handle moves right or down, -1 when it grows left or up.
  direction?: 1 | -1;
  className?: string;
  onResize: (size: number) => void;
  onEnd?: () => void;
  /** The guided tour's anchor id. */
  "data-tour"?: string;
}) {
  const vertical = axis === "x";
  return (
    <div
      role="separator"
      data-tour={tour}
      tabIndex={0}
      aria-orientation={vertical ? "vertical" : "horizontal"}
      aria-label={label}
      aria-valuenow={Math.round(size)}
      aria-valuemin={Math.round(min)}
      aria-valuemax={Math.round(max)}
      className={cx(
        "relative z-10 shrink-0 bg-transparent hover:bg-edge/60 focus-visible:bg-accent/60 focus-visible:outline-none",
        vertical ? "-mx-1 w-2 cursor-col-resize" : "-my-1 h-2 cursor-row-resize",
        className,
      )}
      onPointerDown={(e) =>
        startEdgeDrag(e, { startWidth: size, min, max, direction, axis, onResize, onEnd })
      }
      onKeyDown={(e) => {
        const next = keyedSize(e.key, e.shiftKey, { size, min, max, direction, axis });
        if (next === null) return;
        e.preventDefault();
        onResize(next);
        onEnd?.();
      }}
    />
  );
}

export function TabStrip<K extends string>({
  tabs,
  value,
  onChange,
  className,
}: {
  tabs: ReadonlyArray<{ key: K; label: ReactNode }>;
  value: K;
  onChange: (key: K) => void;
  className?: string;
}) {
  return (
    <div
      role="tablist"
      className={cx("flex h-9 shrink-0 items-center gap-1.5 border-b border-edge", className)}
    >
      {tabs.map((t) => (
        <button
          key={t.key}
          type="button"
          role="tab"
          aria-selected={t.key === value}
          onClick={() => onChange(t.key)}
          className={cx(
            "rounded-md px-2 py-1 text-ui font-medium",
            t.key === value ? "bg-panel-2 text-ink" : "text-ink-dim hover:text-ink",
          )}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}

const CALLOUT_TONE = {
  info: { box: "border-edge bg-panel-2", icon: "text-ink-dim" },
  warn: { box: "border-warn/35 bg-warn/10", icon: "text-warn-ink" },
  bad: { box: "border-bad/35 bg-bad/10", icon: "text-bad" },
} as const;

export function Callout({
  tone,
  icon,
  children,
}: {
  tone: keyof typeof CALLOUT_TONE;
  icon: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className={cx("flex gap-2.5 rounded-lg border p-3 text-ui text-ink-dim", CALLOUT_TONE[tone].box)}>
      <span className={cx("shrink-0 pt-px", CALLOUT_TONE[tone].icon)}>{icon}</span>
      <span className="text-pretty">{children}</span>
    </div>
  );
}

export function StepList({ children }: { children: ReactNode }) {
  return <div className="divide-y divide-edge/60 rounded-md border border-edge bg-panel-2/40">{children}</div>;
}

export function StepRow({
  label,
  status,
  detail,
}: {
  label: string;
  status?: StepStatus;
  detail?: string | null;
}) {
  const shown = status === "failed" ? "failed" : status === "skipped" ? "skipped" : detail;
  return (
    <div className="flex h-9 items-center gap-2.5 px-3">
      <span className="flex h-4 w-4 shrink-0 items-center justify-center">
        {status === "done" ? (
          <Icon name="check" size="sm" className="text-ok" />
        ) : status === "running" ? (
          <Spinner size="sm" />
        ) : status === "failed" ? (
          <Icon name="close" size="xs" className="text-bad-ink" />
        ) : (
          <span aria-hidden="true" className="h-[5px] w-[5px] rounded-full bg-edge" />
        )}
      </span>
      <span
        className={cx(
          "min-w-0 flex-1 truncate text-body",
          status === "running" || status === "failed" ? "text-ink" : "text-ink-dim",
          status === undefined || status === "skipped" ? "opacity-60" : null,
        )}
      >
        {label}
      </span>
      {shown && (
        <span
          className={cx(
            "shrink-0 truncate text-ui",
            status === "failed" ? "font-mono text-bad" : "font-mono text-ink-dim",
            status === "skipped" && "font-sans opacity-60",
          )}
          style={{ maxWidth: 190 }}
        >
          {shown}
        </span>
      )}
    </div>
  );
}
