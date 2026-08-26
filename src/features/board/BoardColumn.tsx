import type { ReactNode } from "react";
import { ModeIcon } from "@/components/icons";
import { MODE_STYLES, type StageKey } from "@/lib/domain/stages";

export function BoardColumn({
  label,
  stageKey,
  count,
  children,
}: {
  label: string;
  stageKey: StageKey;
  count: number;
  children: ReactNode;
}) {
  const subtitle = count === 0 ? "No workstreams" : `${count} workstream${count === 1 ? "" : "s"}`;
  return (
    <div className="flex min-h-0 flex-col rounded-xl border border-edge/50 bg-panel-2/40 p-5">
      <div className="mb-3 flex items-center gap-3">
        <span
          className={`flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-edge bg-panel-2 ${MODE_STYLES[stageKey].text}`}
        >
          <ModeIcon mode={stageKey} size={18} />
        </span>
        <div className="min-w-0">
          <span className="block text-base font-semibold text-ink">{label}</span>
          <span className="mt-0.5 block text-xs text-ink-dim">{subtitle}</span>
        </div>
      </div>
      <div
        role="list"
        aria-label={`${label}, ${subtitle}`}
        className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto pr-0.5"
      >
        {children}
      </div>
    </div>
  );
}
