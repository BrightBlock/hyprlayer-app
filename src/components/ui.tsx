import { useEffect } from "react";
import type {
  ButtonHTMLAttributes,
  HTMLAttributes,
  InputHTMLAttributes,
  ReactNode,
  SelectHTMLAttributes,
} from "react";

function cx(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(" ");
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
        variant === "primary" && "border border-edge bg-accent text-ink hover:bg-panel-2",
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

export function Select({ className, ...rest }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select
      className={cx(
        "w-full rounded-md border border-edge bg-panel-2 px-2 py-1.5 text-sm text-ink",
        "focus:border-ink-dim/70 focus:outline-none disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...rest}
    />
  );
}

export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="block text-sm">
      <span className="mb-1 block text-xs font-medium text-ink-dim">{label}</span>
      {children}
    </label>
  );
}

type DialogProps = {
  open: boolean;
  onClose: () => void;
  title?: ReactNode;
  children: ReactNode;
  className?: string;
};

/** Dependency-free modal: fixed overlay + panel; Escape or a backdrop click closes. */
export function Dialog({ open, onClose, title, children, className }: DialogProps) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

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
        className={cx(
          "max-h-full w-full max-w-md overflow-y-auto rounded-lg border border-edge bg-panel p-4 shadow-2xl",
          className,
        )}
      >
        {title && <h2 className="mb-3 text-sm font-semibold tracking-wide text-ink">{title}</h2>}
        {children}
      </div>
    </div>
  );
}

export function ErrorNote({ error }: { error: unknown }) {
  const message = error instanceof Object && "message" in error ? String((error as { message: unknown }).message) : String(error);
  const remedy =
    error instanceof Object && "remedy" in error && (error as { remedy: unknown }).remedy
      ? String((error as { remedy: unknown }).remedy)
      : null;
  return (
    <div className="selectable rounded-md border border-bad/30 bg-bad/10 p-3 text-sm">
      <p className="text-bad">{message}</p>
      {remedy && <p className="mt-1 text-ink-dim">{remedy}</p>}
    </div>
  );
}
