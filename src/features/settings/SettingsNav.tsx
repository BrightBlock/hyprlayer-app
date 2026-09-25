import { Link } from "@tanstack/react-router";

import { Icon, type IconName } from "@/components/Icon";
import { cx, RowSpine } from "@/components/ui";

import type { SettingsSection } from "./sections";

type NavEntry =
  | { id: SettingsSection; label: string; icon: IconName }
  | { href: "/agents"; label: string; icon: IconName };

const NAV: NavEntry[] = [
  { id: "general", label: "General", icon: "sliders" },
  { id: "harnesses", label: "Harnesses", icon: "agents" },
  // Workshop is the Agents page; the entry leaves Settings.
  { href: "/agents", label: "Workshop", icon: "workshop" },
  { id: "git", label: "Git", icon: "branch" },
  { id: "notifications", label: "Notifications", icon: "notifications" },
  { id: "keyboard", label: "Keyboard", icon: "keyboard" },
  { id: "appearance", label: "Appearance", icon: "appearance" },
  { id: "servers", label: "Servers", icon: "server" },
  { id: "advanced", label: "Advanced", icon: "plugin" },
  { id: "binaries", label: "Binaries", icon: "console" },
];

const ROW =
  "relative flex h-[34px] w-full items-center gap-2.5 overflow-hidden rounded-control px-3 text-left text-body transition-colors " +
  "focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60";
const IDLE = "text-ink-2 hover:bg-edge/50 hover:text-ink";

export function SettingsNav({
  section,
  onSection,
  serverCount,
}: {
  section: SettingsSection;
  onSection: (section: SettingsSection) => void;
  serverCount: number;
}) {
  return (
    <nav
      aria-label="Settings sections"
      className="w-[246px] shrink-0 overflow-y-auto border-r border-edge bg-panel p-2"
    >
      <ul className="space-y-0.5">
        {NAV.map((item) => {
          if ("href" in item) {
            return (
              <li key={item.href}>
                <Link to={item.href} className={cx(ROW, IDLE)}>
                  <Icon name={item.icon} />
                  <span className="flex-1 truncate">{item.label}</span>
                  <Icon name="external" size="xs" className="text-ink-4" />
                </Link>
              </li>
            );
          }
          const selected = section === item.id;
          return (
            <li key={item.id}>
              <button
                type="button"
                onClick={() => onSection(item.id)}
                aria-current={selected ? "page" : undefined}
                className={cx(ROW, selected ? "bg-accent-soft font-semibold text-ink" : IDLE)}
              >
                {selected && <RowSpine />}
                <Icon name={item.icon} />
                <span className="flex-1 truncate">{item.label}</span>
                {item.id === "servers" && (
                  <span className="text-meta text-ink-dim">{serverCount}</span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}
