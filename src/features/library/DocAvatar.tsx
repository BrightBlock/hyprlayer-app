import { BrandMark, harnessBrand } from "@/components/BrandMark";
import { Icon } from "@/components/Icon";
import type { DocAgent } from "@/lib/bindings";
import { initials } from "@/lib/format";
import { docTypeIcon } from "./docTypeIcon";

const CIRCLE = "flex h-7 w-7 shrink-0 items-center justify-center rounded-full";

export function DocAvatar({
  author,
  agent,
  type,
}: {
  author: string | null;
  agent: DocAgent | null;
  type: string | null;
}) {
  if (!author) {
    return (
      <span className={`${CIRCLE} bg-panel-2 text-ink-2`} aria-hidden="true">
        {agent ? (
          <BrandMark brand={harnessBrand(agent.harness)} size="sm" />
        ) : (
          <Icon name={docTypeIcon(type)} size="sm" />
        )}
      </span>
    );
  }
  return (
    // self-start: the row stretches its children, which would drop the badge to the row's bottom.
    <span className="relative h-7 w-7 shrink-0 self-start" aria-hidden="true">
      <span className={`${CIRCLE} bg-ink text-ui-sm font-semibold text-panel`}>{initials(author)}</span>
      {agent && (
        <span className="absolute -bottom-1 -right-1 flex h-4 w-4 items-center justify-center rounded-full bg-panel-2 text-ink-2 ring-2 ring-panel">
          <BrandMark brand={harnessBrand(agent.harness)} size="xs" />
        </span>
      )}
    </span>
  );
}
