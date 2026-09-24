import { Server } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode, type RefObject } from "react";

import { ICON } from "@/components/lucide";
import { cx } from "@/components/ui";
import { toggleServerMenu, useHostsUiStore } from "@/features/hosts/hostsUi";
import { useReconnectStore } from "@/features/hosts/reconnectState";
import { useConnectionStatus } from "@/lib/queries";
import { BAR_BUTTON, Divider, Dot } from "./parts";
import { syncLabel, type SyncTone } from "./statusBarState";

const TONE_DOT: Record<SyncTone, string> = {
  ok: "bg-ok",
  warn: "bg-warn",
  bad: "bg-bad",
};

/** The server connection: its host in the tooltip, a tone dot, and a drop counter while lost. */
export function SyncStatus({
  compact,
  divided,
  menu,
}: {
  compact: boolean;
  divided: boolean;
  menu?: (anchorRef: RefObject<HTMLElement | null>) => ReactNode;
}) {
  const status = useConnectionStatus();
  const reconnect = useReconnectStore();
  const menuOpen = useHostsUiStore((s) => s.menu);
  const anchorRef = useRef<HTMLButtonElement>(null);
  const [now, setNow] = useState(() => Date.now());
  const label = syncLabel(status.isSuccess ? status.data : null, reconnect, now);
  const ticking = label?.ticking ?? false;

  useEffect(() => {
    if (!ticking) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [ticking]);

  if (!label) return null;
  return (
    <>
      {divided && <Divider />}
      <button
        ref={anchorRef}
        type="button"
        onClick={toggleServerMenu}
        title={label.title}
        aria-label={label.title}
        aria-haspopup="dialog"
        aria-expanded={menuOpen && status.data?.state === "connected"}
        className={cx(BAR_BUTTON, "text-ink-dim hover:text-ink")}
      >
        <Server {...ICON} size={14} className="shrink-0" />
        {!compact && <span className="whitespace-nowrap">{label.text}</span>}
        <Dot className={TONE_DOT[label.tone]} />
      </button>
      {menu?.(anchorRef)}
    </>
  );
}
