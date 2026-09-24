import type { ReactNode } from "react";

import { cx } from "@/components/ui";

export const CARD_ROW_SPAN = "col-start-2 col-end-4 min-w-0 empty:hidden";

export function CardRow({
  label,
  align = "start",
  className,
  children,
}: {
  label: string;
  align?: "start" | "center";
  className?: string;
  children: ReactNode;
}) {
  return (
    <div
      className={cx(
        "grid grid-cols-[104px_minmax(0,1fr)_auto] gap-x-4 gap-y-2 px-4 py-[11px]",
        align === "center" ? "items-center" : "items-start",
        className,
      )}
    >
      <div className={cx("text-xs text-ink-dim", align === "start" && "pt-0.5")}>{label}</div>
      {children}
    </div>
  );
}
