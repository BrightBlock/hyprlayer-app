import { useEffect, useState } from "react";

import { HyprlayerMark } from "@/components/Logo";
import { formatElapsed } from "@/lib/format";

import type { Activity } from "./activityStatus";

// formatElapsed shows tenths under a minute, whole seconds after.
function Elapsed({ since }: { since: number }) {
  const [now, setNow] = useState(() => Date.now());
  const fine = now - since < 60_000;
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), fine ? 100 : 1000);
    return () => clearInterval(id);
  }, [fine]);
  return (
    <span aria-hidden className="shrink-0 tabular-nums">
      · {formatElapsed(now - since)}
    </span>
  );
}

export function ActivityBar({ activity }: { activity: Activity | null }) {
  if (!activity) return null;
  const { label, detail, tone, since, more, inline, recap } = activity;
  const labelColor = tone === "wait" ? "text-warn-ink" : tone === "quiet" ? "" : "text-ink";
  return (
    <div role="status" className="mb-1.5 px-1 text-meta text-ink-dim">
      <div className="flex min-w-0 items-center gap-1.5">
        <HyprlayerMark size={14} animated={tone === "work"} tone={tone === "work" ? "accent" : "muted"} />
        <span className={`shrink-0 ${labelColor}`}>{label}</span>
        {detail && (
          <span className="min-w-0 truncate">
            {inline ? "" : "· "}
            {detail}
          </span>
        )}
        {more > 0 && <span className="shrink-0">+{more}</span>}
        {/* Keyed so a switch between turn and wait clocks starts from a fresh `now`. */}
        {since !== null && <Elapsed key={since} since={since} />}
      </div>
      {recap && (
        <p className="selectable mt-1 pl-5">
          <span className="font-medium text-ink">recap:</span> <span className="italic">{recap}</span>
        </p>
      )}
    </div>
  );
}
