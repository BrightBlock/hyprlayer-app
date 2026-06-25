import type { ReactNode } from "react";

/** One labelled settings row: title + description on the left, a control on the
 *  right. Shared across the Settings sections (Git, Notifications) and the
 *  Harnesses tabs so every row reads identically. */
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
