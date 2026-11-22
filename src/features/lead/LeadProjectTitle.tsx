import type { LeadView, WorkstreamView } from "@/lib/bindings";
import { LeadMark } from "./LeadChip";
import { useLeadNeedsYou } from "./lead";

export function LeadProjectTitle({
  lead,
  name,
  views,
}: {
  lead: LeadView;
  name: string;
  views: WorkstreamView[];
}) {
  const needsYou = useLeadNeedsYou(lead, views);

  return (
    <>
      <span className="relative flex shrink-0" title={needsYou > 0 ? `Lead · ${needsYou} needs you` : "Lead"}>
        <LeadMark className="h-[22px] w-[22px]" />
        {needsYou > 0 && (
          <span
            aria-hidden="true"
            className="absolute -right-px -bottom-px h-2 w-2 rounded-full border-[1.5px] border-panel bg-warn"
          />
        )}
      </span>
      <span className="truncate">{name}</span>
    </>
  );
}
