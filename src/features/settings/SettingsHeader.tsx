import { useNavigate } from "@tanstack/react-router";

import { effectiveChord } from "@/lib/keybindings/resolve";
import { IS_MACOS } from "@/lib/platform";
import { useBootstrap } from "@/lib/queries";
import { Icon } from "@/components/Icon";
import { ChordHint } from "@/components/ui";

import type { SettingsSection } from "./sections";
import { serversHeadline } from "./settingsMeta";
import { useServerCounts } from "./useServerCounts";

/** The window header's crumb on /settings; Servers gets a way back to General. */
export function SettingsCrumb({ section }: { section: SettingsSection }) {
  const navigate = useNavigate();
  if (section !== "servers") {
    return <span className="truncate font-medium text-ink">Settings</span>;
  }
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      <button
        type="button"
        onClick={() => void navigate({ to: "/settings", search: {}, replace: true })}
        className="flex shrink-0 items-center gap-0.5 font-medium text-ink-dim hover:text-ink"
      >
        <Icon name="chevronLeft" size="sm" />
        Settings
      </button>
      <span className="text-ink-4">/</span>
      <span className="truncate font-medium text-ink">Servers</span>
    </span>
  );
}

export function SettingsHeaderMeta({ section }: { section: SettingsSection }) {
  const overrides = useBootstrap().data?.settings.keybindings?.overrides ?? {};
  const chord = effectiveChord(overrides, "nav.settings", IS_MACOS);
  if (section === "servers") return <ServersHeadline />;
  return chord ? <ChordHint chord={chord} className="text-meta text-ink-dim" /> : null;
}

function ServersHeadline() {
  return (
    <span className="text-meta text-ink-dim">{serversHeadline(useServerCounts())}</span>
  );
}
