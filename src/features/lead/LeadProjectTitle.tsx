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
      <span className="flex shrink-0" title={needsYou > 0 ? `Lead · ${needsYou} needs you` : "Lead"}>
        <RepoAvatar
          source={source}
          name={name}
          size={22}
          className={
            needsYou > 0
              ? "rounded-md shadow-[0_0_6px_1px_color-mix(in_srgb,var(--color-warn)_45%,transparent)]"
              : "rounded-md"
          }
        />
      </span>
      <span className="truncate">{name}</span>
    </>
  );
}
