import { memo, useState } from "react";

import { Icon } from "@/components/Icon";
import { MarkdownPreview } from "@/features/library/MarkdownPreview";

export const SkillChip = memo(function SkillChip({
  name,
  body,
  toolUseId,
}: {
  name: string;
  body: string | null;
  toolUseId?: string;
}) {
  const [open, setOpen] = useState(false);
  const label = (
    <>
      <Icon name="skill" size="xs" />
      <span className="truncate">{name} · skill</span>
    </>
  );
  const chip =
    "inline-flex max-w-full items-center gap-1.5 rounded-control bg-accent-soft px-2.5 py-1 " +
    "text-meta font-semibold text-ink-2";
  return (
    <div className="text-ui" data-tooluse-id={toolUseId}>
      {body ? (
        <button
          type="button"
          aria-expanded={open}
          title="Show skill instructions"
          onClick={() => setOpen((v) => !v)}
          className={`${chip} hover:text-ink`}
        >
          {label}
        </button>
      ) : (
        <span className={chip}>{label}</span>
      )}
      {open && body && (
        <div className="selectable mt-1 border-l border-edge pl-3">
          <MarkdownPreview body={body} />
        </div>
      )}
    </div>
  );
});
