import { useEffect, type ReactNode } from "react";

import { BLANK_CTA } from "@/components/BlankState";
import { Icon } from "@/components/Icon";
import { Button, cx, Kbd } from "@/components/ui";
import { isEditableTarget } from "@/lib/keybindings/useKeybindings";

export const CARD_CLASS = "bg-panel/90 px-5 py-[18px]";

export function CheckDot({ className }: { className: string }) {
  return (
    <span
      className={cx("flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full text-accent-ink", className)}
    >
      <Icon name="check" size="xs" />
    </span>
  );
}

export type StepProps = {
  title: ReactNode;
  lede?: ReactNode;
  children: ReactNode;
  onBack?: () => void;
  onNext?: () => void;
  nextEnabled?: boolean;
  nextLabel?: string;
  footer?: ReactNode;
};

// Editable targets and buttons already own Enter; without these guards the name input and a focused button would fire twice.
function useEnterToContinue(onNext: (() => void) | null) {
  useEffect(() => {
    if (!onNext) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Enter" || e.repeat || e.isComposing || e.defaultPrevented) return;
      if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
      if (isEditableTarget(e.target)) return;
      if (e.target instanceof HTMLElement && e.target.tagName === "BUTTON") return;
      onNext();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onNext]);
}

export function Step({
  title,
  lede,
  children,
  onBack,
  onNext,
  nextEnabled = true,
  nextLabel = "Next",
  footer,
}: StepProps) {
  useEnterToContinue(onNext && nextEnabled ? onNext : null);

  return (
    <div>
      <h1 className="mb-4 font-serif text-display text-ink">{title}</h1>
      {lede && <p className="mb-6 max-w-[520px] text-body-lg text-ink-dim">{lede}</p>}
      <div>{children}</div>
      <div className="mt-[26px] flex items-center gap-2.5">
        {onBack && (
          <Button
            type="button"
            onClick={onBack}
            className="h-[38px] border-transparent bg-transparent px-2.5 font-semibold text-ink-dim hover:text-ink"
          >
            <Icon name="arrowLeft" size="sm" /> Back
          </Button>
        )}
        <div className="flex-1" />
        {onNext && nextEnabled && (
          <span className="flex items-center gap-1.5 text-ui-sm text-ink-4">
            <Kbd>↵</Kbd> to continue
          </span>
        )}
        {footer ??
          (onNext && (
            <Button variant="primary" className={BLANK_CTA} onClick={onNext} disabled={!nextEnabled}>
              {nextLabel} <Icon name="arrowRight" size="sm" />
            </Button>
          ))}
      </div>
    </div>
  );
}
