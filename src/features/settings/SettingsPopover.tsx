import { useId, type ReactNode, type RefObject } from "react";
import { Link } from "@tanstack/react-router";

import { visibleAgents } from "@/features/agents/agents";
import type { AppSettings } from "@/lib/bindings";
import { effectiveChord } from "@/lib/keybindings/resolve";
import { invalidateNotificationSettingsCache } from "@/lib/notifications";
import { IS_MACOS } from "@/lib/platform";
import { formatBytes } from "@/lib/format";
import {
  useAgentFiles,
  useArchivedWorkstreams,
  useBootstrap,
  useSetAppSettings,
  useSkillFiles,
} from "@/lib/queries";
import { appearanceSegment, useThemeStore, type AppearanceSegment } from "@/lib/theme";
import { Icon } from "@/components/Icon";
import {
  BAR_POPOVER,
  BAR_POPOVER_ROW,
  ChordHint,
  cx,
  ErrorNote,
  Popover,
  SegmentedControl,
  Toggle,
} from "@/components/ui";

import { openArchivedWorktrees } from "@/features/archive/archivedWorktreesStore";
import { setServerAutoUpdate, useServerAutoUpdate } from "@/features/hosts/serverAutoUpdate";

import { HarnessPicker } from "./HarnessPicker";
import { quickSettings, withConfirmQuit, withDefaultHarness, withNotify } from "./settingsPatches";
import { serversMeta, workshopMeta } from "./settingsMeta";
import { useServerCounts } from "./useServerCounts";

const APPEARANCE: Array<{ value: AppearanceSegment; label: string }> = [
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
  { value: "system", label: "Auto" },
];

const ROW = "flex items-center justify-between gap-3 px-3 py-2 text-ui text-ink-2";

export function SettingsPopover({
  open,
  onClose,
  anchorRef,
}: {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLElement | null>;
}) {
  const headingId = useId();
  return (
    <Popover
      open={open}
      onClose={onClose}
      anchorRef={anchorRef}
      align="end"
      sideOffset={10}
      caret
      trapFocus
      aria-labelledby={headingId}
      className={BAR_POPOVER}
    >
      <PopoverBody headingId={headingId} onClose={onClose} />
    </Popover>
  );
}

function PopoverBody({ headingId, onClose }: { headingId: string; onClose: () => void }) {
  const settings = useBootstrap().data?.settings;
  const chord = effectiveChord(settings?.keybindings?.overrides ?? {}, "nav.settings", IS_MACOS);
  return (
    <>
      <div className="flex items-center justify-between px-3 pb-1.5 pt-1">
        <span id={headingId} className="lbl">
          Settings
        </span>
        {chord && <ChordHint chord={chord} className="text-meta text-ink-dim" />}
      </div>
      <AppearanceRow />
      {settings ? <ServerBackedRows settings={settings} /> : null}
      <ServerAutoUpdateRow />
      <div className="mx-1 my-1 h-px bg-edge-2" />
      <WorkshopLink onClose={onClose} />
      <Link to="/settings" search={{ section: "servers" }} onClick={onClose} className={BAR_POPOVER_ROW}>
        <Icon name="server" size="sm" className="text-ink-dim" />
        <span className="flex-1">Servers</span>
        <Meta>{serversMeta(useServerCounts())}</Meta>
      </Link>
      <ArchivedLink onClose={onClose} />
      <Link
        to="/settings"
        search={{}}
        onClick={onClose}
        className={cx(BAR_POPOVER_ROW, "mt-0.5 bg-panel-2 font-semibold text-ink hover:bg-panel-2")}
      >
        <span className="flex-1">All settings</span>
        <Icon name="chevronRight" size="sm" className="text-ink-dim" />
      </Link>
    </>
  );
}

function AppearanceRow() {
  const mode = useThemeStore((s) => s.mode);
  const setMode = useThemeStore((s) => s.setMode);
  return (
    <div className={ROW}>
      <span>Appearance</span>
      <SegmentedControl
        aria-label="Appearance"
        value={appearanceSegment(mode)}
        options={APPEARANCE}
        onChange={setMode}
      />
    </div>
  );
}

function ServerBackedRows({ settings }: { settings: AppSettings }) {
  const save = useSetAppSettings();
  const quick = quickSettings(settings);
  return (
    <>
      <div className={ROW}>
        <span>Default harness</span>
        <HarnessPicker
          variant="dropdown"
          value={quick.defaultHarness}
          onChange={(key) => save.mutate(withDefaultHarness(settings, key))}
        />
      </div>
      <Toggle
        className={cx(ROW, "flex w-full")}
        checked={quick.notify}
        onChange={(on) =>
          save.mutate(withNotify(settings, on), {
            onSuccess: () => invalidateNotificationSettingsCache(),
          })
        }
      >
        <span className="flex-1">Notify when a session needs me</span>
      </Toggle>
      <Toggle
        className={cx(ROW, "flex w-full")}
        checked={quick.confirmQuit}
        onChange={(on) => save.mutate(withConfirmQuit(settings, on))}
      >
        <span className="flex-1">Confirm before quitting</span>
      </Toggle>
      {save.isError && (
        <div className="px-2 pb-1">
          <ErrorNote error={save.error} onDismiss={() => save.reset()} />
        </div>
      )}
    </>
  );
}

function ServerAutoUpdateRow() {
  const enabled = useServerAutoUpdate((s) => s.enabled);
  return (
    <Toggle className={cx(ROW, "flex w-full")} checked={enabled} onChange={setServerAutoUpdate}>
      <span className="flex-1">Update server when idle</span>
    </Toggle>
  );
}

function WorkshopLink({ onClose }: { onClose: () => void }) {
  const files = useAgentFiles(null).data;
  const skills = useSkillFiles(null).data;
  const agents = files
    ? visibleAgents(files, { tab: "yours", scope: "global", search: "" }).length
    : null;
  return (
    <Link to="/workshop" onClick={onClose} className={BAR_POPOVER_ROW}>
      <Icon name="workshop" size="sm" className="text-ink-dim" />
      <span className="flex-1">Workshop</span>
      <Meta>{workshopMeta(agents, skills?.length ?? null)}</Meta>
    </Link>
  );
}

function ArchivedLink({ onClose }: { onClose: () => void }) {
  const archived = useArchivedWorkstreams().data;
  const bytes = archived?.reduce((sum, a) => sum + (a.info.sizeBytes ?? 0), 0) ?? 0;
  return (
    <button
      type="button"
      aria-haspopup="dialog"
      onClick={() => {
        onClose();
        openArchivedWorktrees();
      }}
      className={cx(BAR_POPOVER_ROW, "w-full text-left")}
    >
      <Icon name="archive" size="sm" className="text-ink-dim" />
      <span className="flex-1">Archived worktrees</span>
      {archived && (
        <Meta>{bytes > 0 ? `${archived.length} · ${formatBytes(bytes)}` : archived.length}</Meta>
      )}
      <Icon name="chevronRight" size="sm" className="text-ink-dim" />
    </button>
  );
}

function Meta({ children }: { children: ReactNode }) {
  return <span className="text-meta text-ink-dim">{children}</span>;
}
