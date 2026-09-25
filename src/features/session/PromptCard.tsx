import { memo, useState } from "react";

import { ZapIcon } from "@/components/icons";
import { STEP_ROW_ON_PANEL, StepChevron } from "./stepRow";

export const PromptCard = memo(function PromptCard({
  name,
  body,
}: {
  name: string;
  body: string;
}) {
  const [open, setOpen] = useState(false);
  const lines = body.split("\n").length;

  return (
    <div className="text-ui">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className={STEP_ROW_ON_PANEL}
      >
        <ZapIcon size={12} className="shrink-0 text-accent" />
        <span className="min-w-0 truncate font-medium text-ink" title={name}>
          {name}
        </span>
        <span className="shrink-0 text-ink-dim">
          {lines} {lines === 1 ? "line" : "lines"}
        </span>
        <StepChevron open={open} size={11} />
      </button>
      {open && (
        <p className="selectable mt-1 whitespace-pre-wrap break-words border-l border-edge pl-3 text-body text-ink">
          {body}
        </p>
      )}
    </div>
  );
});
