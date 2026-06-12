import type { ButtonHTMLAttributes, HTMLAttributes, ReactNode } from "react";

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
        variant === "primary" && "bg-accent/90 text-surface hover:bg-accent",
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

type BadgeTone = "ok" | "warn" | "bad" | "dim" | "accent";

export function Badge({ tone = "dim", children }: { tone?: BadgeTone; children: ReactNode }) {
  const tones: Record<BadgeTone, string> = {
    ok: "bg-ok/15 text-ok",
    warn: "bg-warn/15 text-warn",
    bad: "bg-bad/15 text-bad",
    dim: "bg-edge/60 text-ink-dim",
    accent: "bg-accent/15 text-accent",
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
