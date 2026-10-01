import { Icon } from "@/components/Icon";
import { initialsOf } from "@/lib/format";
import { useTranscriptSync } from "./syncStore";

const MAX_AVATARS = 3;

/** Who else has this session open; only shown once a second client is viewing it. */
export function SessionPresenceBadge({ sessionId }: { sessionId?: string | null }) {
  const viewers = useTranscriptSync((s) => (sessionId ? s.viewers[sessionId] : undefined));
  if (!viewers || viewers.length < 2) return null;
  return (
    <span
      className="flex shrink-0 items-center gap-1.5"
      title={viewers.map((v) => v.device).join(", ")}
    >
      <Icon name="device" size="xs" className="text-ink-dim" />
      <span className="flex items-center">
        {viewers.slice(0, MAX_AVATARS).map((viewer, i) => (
          <span
            key={viewer.clientId}
            aria-hidden
            className={`flex h-5 w-5 items-center justify-center rounded-full bg-panel-2 text-tag leading-none text-ink-2 ring-2 ring-panel ${
              i > 0 ? "-ml-1.5" : ""
            }`}
          >
            {initialsOf(viewer.device.replace(/-/g, " "))}
          </span>
        ))}
      </span>
      <span className="text-meta text-ink-dim">{viewers.length} clients</span>
    </span>
  );
}
