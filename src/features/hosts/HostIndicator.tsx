import { GlobeIcon } from "@/components/icons";
import { RAIL_ICON_BASE, railNavClass } from "@/components/ui";
import { useConnectionStatus } from "@/lib/queries";
import { openHostsUi } from "./hostsUi";

type Tone = "accent" | "bad" | "warn";

const TONE_CLASS: Record<Tone, string> = {
  accent: "bg-accent/15 text-accent ring-[1.5px] ring-inset ring-accent/40",
  bad: "bg-bad/15 text-bad ring-[1.5px] ring-inset ring-bad/40",
  warn: "bg-warn/15 text-warn ring-[1.5px] ring-inset ring-warn/40",
};

const TONE_DOT: Record<Tone, string> = {
  accent: "bg-accent",
  bad: "bg-bad",
  warn: "bg-warn",
};

const TONE_SHADOW: Record<Tone, string> = {
  accent: "0 0 0 2px var(--color-surface), 0 0 6px var(--color-accent)",
  bad: "0 0 0 2px var(--color-surface), 0 0 6px var(--color-bad)",
  warn: "0 0 0 2px var(--color-surface), 0 0 6px var(--color-warn)",
};

// Rail icon for the always-on connection state, next to Archive/Scripts.
// Rendered unconditionally so the rail never reflows once the app opens; the
// pre-resolve render matches the quiet local-connected appearance.
export function HostIndicator() {
  const status = useConnectionStatus();
  const connection = status.isSuccess ? status.data : null;
  const label = connection?.host
    ? connection.host.label || connection.host.destination
    : (connection?.localServerName ?? null);

  let tone: Tone | null = null;
  let title = "Server: this machine";

  if (connection?.state === "failed") {
    tone = "bad";
    title = `Not connected to ${label ?? "this machine"}`;
  } else if (connection?.state === "lost") {
    tone = "warn";
    title = `Connection to ${label ?? "this machine"} lost`;
  } else if (connection?.serverOutdated) {
    tone = "warn";
    title = `Server update available on ${label ?? "this machine"}`;
  } else if (label) {
    tone = "accent";
    title = `Server: ${label}`;
  }

  const className = tone
    ? `${RAIL_ICON_BASE} ${TONE_CLASS[tone]}`
    : railNavClass(false);

  return (
    <button
      type="button"
      onClick={openHostsUi}
      className={className}
      title={title}
      aria-label={title}
    >
      <GlobeIcon size={16} />
      {tone && (
        <span
          aria-hidden
          className={`absolute right-1 top-1 h-2 w-2 rounded-full ${TONE_DOT[tone]}`}
          style={{ boxShadow: TONE_SHADOW[tone] }}
        />
      )}
    </button>
  );
}
