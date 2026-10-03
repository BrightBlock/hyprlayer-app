import type { ReactNode } from "react";

import { BlankArt, type BlankArtName } from "./BlankArt";
import { cx } from "./ui";

export const BLANK_CTA = "h-[38px] px-4 font-semibold";
export const BLANK_CTA_SECONDARY = "h-[38px] px-[15px] font-semibold bg-panel/70 text-ink-2";

type Props = {
  art: BlankArtName;
  mirror?: boolean;
  align: "start" | "center";
  eyebrow?: ReactNode;
  title: ReactNode;
  lede: ReactNode;
  actions: ReactNode;
  hint?: ReactNode;
  footer?: ReactNode;
};

export function BlankState({ art, mirror, align, eyebrow, title, lede, actions, hint, footer }: Props) {
  const center = align === "center";
  return (
    <div className="relative flex h-full min-h-0 items-center overflow-hidden bg-surface">
      <BlankArt art={art} fade="pane" mirror={mirror} />
      <div className={cx("relative z-[1]", center ? "mx-auto max-w-[680px] px-10 text-center" : "max-w-[760px] px-24")}>
        {eyebrow && <div className="mb-[22px] text-ui-sm font-semibold text-ink-dim">{eyebrow}</div>}
        <h1 className="mb-[18px] font-serif text-display text-ink">{title}</h1>
        <p className={cx("mb-[30px] max-w-[520px] text-body-lg text-ink-dim", center && "mx-auto")}>{lede}</p>
        <div className={cx("flex items-center gap-2.5", center && "justify-center")}>{actions}</div>
        {hint && (
          <div className={cx("mt-[26px] flex items-center gap-2 text-ui text-ink-dim", center && "justify-center")}>
            {hint}
          </div>
        )}
        {footer}
      </div>
    </div>
  );
}
