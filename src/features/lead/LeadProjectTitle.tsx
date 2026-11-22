import type { LeadView, WorkstreamView } from "@/lib/bindings";
import { RepoAvatar } from "@/components/RepoAvatar";
import { useLeadNeedsYou } from "./lead";

export function LeadProjectTitle({
  lead,
  name,
  source,
  views,
}: {
  lead: LeadView;
  name: string;
  source: string | null;
  views: WorkstreamView[];
}) {
  const needsYou = useLeadNeedsYou(lead, views);

  return (
    <>
      <span className="relative flex shrink-0" title={needsYou > 0 ? `Lead · ${needsYou} needs you` : "Lead"}>
        <RepoAvatar source={source} name={name} size={22} className="rounded-md" />
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
