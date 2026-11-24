import { useEffect, useRef, useState, type ReactNode, type RefObject } from "react";

import { Icon } from "@/components/Icon";
import { cx, Spinner } from "@/components/ui";
import {
  openHostsUi,
  openServerUpdate,
  toggleServerMenu,
  useHostsUiStore,
} from "@/features/hosts/hostsUi";
import { useReconnectStore } from "@/features/hosts/reconnectState";
import { useServerAutoUpdate } from "@/features/hosts/serverAutoUpdate";
import { connectionName } from "@/lib/domain/hosts";
import { useConnectionStatus } from "@/lib/queries";
import { BAR_BUTTON, BAR_PAD_X, Divider, Dot } from "./parts";
import { ServerStatsReadout, serverStatsSummary, useServerStats } from "./ServerStats";
import { useServerStatsShown } from "./serverStatsShown";
import { syncLabel, type SyncTone } from "./statusBarState";

const TONE_DOT: Record<SyncTone, string> = {
  ok: "bg-ok",
  warn: "bg-warn",
  bad: "bg-bad",
};

const TONE_ICON: Record<SyncTone, string | undefined> = {
  ok: undefined,
  warn: "text-warn",
  bad: "text-bad-ink",
};

/** The server connection: its host in the tooltip, a tone dot, a drop counter while lost, and a pending update. */
export function SyncStatus({
  compact,
  divided,
  menu,
  leading,
}: {
  compact: boolean;
  divided: boolean;
  menu?: (anchorRef: RefObject<HTMLElement | null>) => ReactNode;
  /** Sits flush against the Server button, sharing the divider to its left. */
  leading?: ReactNode;
}) {
  const status = useConnectionStatus();
  const reconnect = useReconnectStore();
  const updating = useServerAutoUpdate((s) => s.updating);
  const menuOpen = useHostsUiStore((s) => s.menu);
  const anchorRef = useRef<HTMLButtonElement>(null);
  const [now, setNow] = useState(() => Date.now());
  const label = syncLabel(status.isSuccess ? status.data : null, reconnect, now, updating);
  const connected = status.data?.state === "connected";
  const stats = useServerStats(useServerStatsShown((s) => s.enabled) && connected && !updating);
  const ticking = label?.ticking ?? false;

  useEffect(() => {
    if (!ticking) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [ticking]);

  if (!label) return null;
  const summary = stats ? serverStatsSummary(stats) : "";
  const title = summary ? `${label.title} · ${summary}` : label.title;
  const server = (
    <button
      ref={anchorRef}
      type="button"
      // The menu needs a live server; while down, the server dialog says why and what to do.
      onClick={connected ? toggleServerMenu : openHostsUi}
      title={title}
      aria-label={title}
      aria-haspopup="dialog"
      aria-expanded={menuOpen && connected}
      className={cx(BAR_BUTTON, "text-ink-dim hover:text-ink")}
    >
      {updating ? (
        <Spinner size="xs" />
      ) : (
        <Icon name="server" size="xs" className={TONE_ICON[label.tone]} />
      )}
      {!compact && <span className="whitespace-nowrap">{label.text}</span>}
      <Dot className={TONE_DOT[label.tone]} />
      {stats && <ServerStatsReadout view={stats} compact={compact} />}
    </button>
  );
  const outdated = !updating && connected && status.data?.serverOutdated;
  const where = connectionName(status.data) ?? "this machine";
  return (
    <>
      {divided && <Divider />}
      {leading ? (
        <div className="flex items-center">
          {leading}
          {server}
        </div>
      ) : (
        server
      )}
      {outdated && (
        <button
          type="button"
          onClick={openServerUpdate}
          title={`Update the server on ${where}`}
          // Runs to the bar's right edge, through its padding.
          style={{ marginRight: -BAR_PAD_X }}
          className={cx(
            BAR_BUTTON,
            "h-auto self-stretch rounded-none bg-warn/15 px-3 font-semibold text-warn-ink",
            "enabled:hover:bg-warn/25 enabled:focus-visible:bg-warn/25",
          )}
        >
          <Icon name="update" size="xs" />
          <span className="whitespace-nowrap">{compact ? "Update" : "Update server"}</span>
        </button>
      )}
      {menu?.(anchorRef)}
    </>
  );
}
