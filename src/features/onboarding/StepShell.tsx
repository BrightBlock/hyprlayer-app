import type { ReactNode } from "react";

import { HyprlayerLogo } from "@/components/Logo";

export type StepProps = {
  number: number;
  total: number;
  title: string;
  children: ReactNode;
  footer?: ReactNode;
};

export function Step({ number, total, title, children, footer }: StepProps) {
  return (
    <div className="flex min-h-full flex-col gap-6">
      <header className="mb-2 flex flex-col items-center gap-4 text-center">
        <HyprlayerLogo height={60} />
        <p className="text-xs font-medium uppercase tracking-[0.25em] text-ink-dim">
          Setup {number} of {total}
        </p>
        <h1 className="text-2xl font-semibold tracking-tight text-ink">{title}</h1>
      </header>
      <div className="flex-1">{children}</div>
      {footer && <div className="flex justify-end gap-2 pt-2">{footer}</div>}
    </div>
  );
}
