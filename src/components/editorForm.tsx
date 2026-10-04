import type { ReactNode } from "react";

export function EditorSection({
  title,
  description,
  action,
  children,
  className,
}: {
  title?: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <section className={`border-t border-edge pt-5 first:border-t-0 first:pt-0 ${className ?? ""}`}>
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

export function FormRow({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <div className="grid gap-x-4 gap-y-1.5 py-2 @[560px]:grid-cols-[168px_minmax(0,1fr)]">
      <div className="pt-1.5 text-ui text-ink-dim">{label}</div>
      <div className="min-w-0">
        {children}
        {hint && <p className="mt-1.5 text-ui-sm text-ink-dim">{hint}</p>}
      </div>
    </div>
  );
}
