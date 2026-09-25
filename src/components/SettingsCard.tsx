import type { HTMLAttributes, ReactNode } from "react";

import { cx } from "./ui";

export const SETTINGS_CARD_TITLE = "text-title text-ink";
export const SETTINGS_CARD_DESCRIPTION = "text-ui leading-prose text-ink-dim";

/** SettingsCard's title, for settings cards laid out by hand. */
export function SettingsCardTitle({ className, ...rest }: HTMLAttributes<HTMLHeadingElement>) {
  return <h2 className={cx("mb-3", SETTINGS_CARD_TITLE, className)} {...rest} />;
}

/** v4 settings card. `flush` hands the body to rows that bring their own padding and rules. */
export function SettingsCard({
  title,
  description,
  actions,
  flush = false,
  className,
  children,
}: {
  title?: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  flush?: boolean;
  className?: string;
  children?: ReactNode;
}) {
  const header = title != null || actions != null;
  return (
    <section className={cx("rounded-card border border-edge bg-panel", className)}>
      {header && (
        <header className={cx("flex items-start justify-between gap-4 px-4 pt-4", !children && "pb-4")}>
          <div className="min-w-0">
            {title != null && <h2 className={SETTINGS_CARD_TITLE}>{title}</h2>}
            {description != null && (
              <p className={cx("mt-1", SETTINGS_CARD_DESCRIPTION)}>{description}</p>
            )}
          </div>
          {actions != null && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
        </header>
      )}
      {children != null &&
        (flush ? (
          <div className={cx(header && "mt-3")}>{children}</div>
        ) : (
          <div className={cx("px-4 pb-4", header ? "pt-3" : "pt-4")}>{children}</div>
        ))}
    </section>
  );
}
