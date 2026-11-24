import { useEffect, useId, type RefObject } from "react";
import { Link } from "@tanstack/react-router";

import type { HostsFile } from "@/lib/bindings";
import { connectionName, serverRows, type ServerRow } from "@/lib/domain/hosts";
import {
  useAllWorkstreams,
  useConnectionStatus,
  useHostProbes,
  useHosts,
  useSaveHosts,
} from "@/lib/queries";
import { Icon } from "@/components/Icon";
import { BAR_POPOVER, BAR_POPOVER_ROW, cx, ErrorNote, Popover } from "@/components/ui";
import { useServerStatsShown } from "@/features/statusbar/serverStatsShown";
import { closeServerMenu, openServerMenu, openServerUpdate, useHostsUiStore } from "./hostsUi";
import { ServerHostStats } from "./ServerHostStats";
import { useServerAutoUpdate } from "./serverAutoUpdate";
import { ServerSummary, worktreeCount } from "./ServerSummary";
import { SwitchServerConfirm, useServerSwitch } from "./SwitchServerConfirm";
import { useHostChecks } from "./useHostChecks";

const EMPTY: HostsFile = { hosts: [], active: null };

/** The status bar's server switcher; the Servers settings page keeps add, edit and install. */
export function ServerMenu({ anchorRef }: { anchorRef: RefObject<HTMLElement | null> }) {
  const open = useHostsUiStore((s) => s.menu);
  const connection = useConnectionStatus().data;
  const save = useSaveHosts();
  // A failed switch reopens the menu, which is where its error shows.
  const serverSwitch = useServerSwitch({
    mutateAsync: (file) => save.mutateAsync(file, { onError: openServerMenu }),
  });
  // Closed while disconnected, when the status bar opens HostsDialog instead.
  const connected = connection?.state === "connected";
  const shown = open && connected;
  const headingId = useId();
  const statsShown = useServerStatsShown((s) => s.enabled);

  useEffect(() => {
    if (open && !connected) closeServerMenu();
  }, [open, connected]);

  return (
    <>
      <Popover
        open={shown}
        onClose={closeServerMenu}
        anchorRef={anchorRef}
        side="top"
        align="start"
        sideOffset={10}
        caret
        trapFocus
        aria-labelledby={headingId}
        // The host meters' captions need the Lead panel's width to stay on one line.
        className={cx(BAR_POPOVER, statsShown && "w-[360px]")}
      >
        {statsShown && (
          <>
            <ServerHostStats connection={connection} />
            <div className="mx-1 mb-1.5 h-px bg-edge-2" />
          </>
        )}
        <div className="px-3 pb-1.5 pt-1">
          <span id={headingId} className="lbl">
            Servers
          </span>
        </div>
        <ServerRows
          onSwitch={(file) => {
            closeServerMenu();
            serverSwitch.request(file);
          }}
          saving={save.isPending}
        />
        {save.isError && (
          <div className="px-2 pb-1">
            <ErrorNote error={save.error} onDismiss={() => save.reset()} />
          </div>
        )}
        <div className="mx-1 my-1 h-px bg-edge-2" />
        <Link
          to="/settings"
          search={{ section: "servers" }}
          onClick={closeServerMenu}
          className={cx(BAR_POPOVER_ROW, "bg-panel-2 font-semibold text-ink hover:bg-panel-2")}
        >
          <span className="flex-1">Manage servers</span>
          <Icon name="chevronRight" size="sm" className="text-ink-dim" />
        </Link>
      </Popover>
      <SwitchServerConfirm
        pending={serverSwitch.pending}
        leaving={connectionName(connection) ?? "this machine"}
        live={connection?.state !== "failed"}
        busy={save.isPending}
        onCancel={serverSwitch.cancel}
        onConfirm={() => void serverSwitch.confirm()}
      />
    </>
  );
}

function ServerRows({
  onSwitch,
  saving,
}: {
  onSwitch: (file: HostsFile) => void;
  saving: boolean;
}) {
  const file = useHosts().data ?? EMPTY;
  const connection = useConnectionStatus().data;
  const checks = useHostChecks(file.active ?? null, useHostProbes()).checks;
  const worktrees = worktreeCount(useAllWorkstreams().data);
  const rows = serverRows(file, connection, checks);
  const updating = useServerAutoUpdate((s) => s.updating);
  const outdated = connection?.state === "connected" && connection.serverOutdated && !updating;

  // As the Servers page's "Set default": switching is what makes a server the default.
  const canSwitch = (row: ServerRow) => row.state !== "unreachable" && !row.isDefault;

  return (
    <>
      <ul aria-label="Servers">
        {rows.map((row) => (
          <li key={row.key}>
            <button
              type="button"
              disabled={saving || !canSwitch(row)}
              aria-current={row.isConnected || undefined}
              title={canSwitch(row) ? `Switch to ${row.name}` : undefined}
              onClick={() => onSwitch({ ...file, active: row.host?.destination ?? null, localProfile: null })}
              className={cx(
                "flex w-full items-center gap-3 rounded-control px-3 py-2 text-left",
                "focus:outline-none enabled:hover:bg-edge/50 enabled:focus-visible:bg-edge/50",
                "disabled:cursor-default",
              )}
            >
              <ServerSummary
                row={row}
                check={row.host ? checks[row.key] : undefined}
                connection={connection}
                worktrees={row.isConnected ? worktrees : null}
              />
            </button>
          </li>
        ))}
      </ul>
      {outdated && (
        <button type="button" onClick={openServerUpdate} className={cx(BAR_POPOVER_ROW, "w-full")}>
          <span className="flex-1 text-left text-warn-ink">Server update available</span>
          <Icon name="chevronRight" size="sm" className="text-ink-dim" />
        </button>
      )}
    </>
  );
}
