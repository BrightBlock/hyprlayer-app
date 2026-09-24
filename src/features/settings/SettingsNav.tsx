import { Link } from "@tanstack/react-router";
import {
  Bell,
  Bot,
  ExternalLink,
  GitBranch,
  Hammer,
  Keyboard,
  Palette,
  Puzzle,
  Server,
  SlidersHorizontal,
  SquareTerminal,
  type LucideIcon,
} from "lucide-react";

import { ICON } from "@/components/lucide";
import { cx, RowSpine } from "@/components/ui";

import type { SettingsSection } from "./sections";

type NavEntry =
  | { id: SettingsSection; label: string; icon: LucideIcon }
  | { href: "/agents"; label: string; icon: LucideIcon };

const NAV: NavEntry[] = [
  { id: "general", label: "General", icon: SlidersHorizontal },
  { id: "harnesses", label: "Harnesses", icon: Bot },
  // Workshop is the Agents page; the entry leaves Settings.
  { href: "/agents", label: "Workshop", icon: Hammer },
  { id: "git", label: "Git", icon: GitBranch },
  { id: "notifications", label: "Notifications", icon: Bell },
  { id: "keyboard", label: "Keyboard", icon: Keyboard },
  { id: "appearance", label: "Appearance", icon: Palette },
  { id: "servers", label: "Servers", icon: Server },
  { id: "advanced", label: "Advanced", icon: Puzzle },
  { id: "binaries", label: "Binaries", icon: SquareTerminal },
];

const ROW =
  "relative flex h-[34px] w-full items-center gap-2.5 overflow-hidden rounded-control px-3 text-left text-[13px] transition-colors " +
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
          const Icon = item.icon;
          if ("href" in item) {
            return (
              <li key={item.href}>
                <Link to={item.href} className={cx(ROW, IDLE)}>
                  <Icon {...ICON} />
                  <span className="flex-1 truncate">{item.label}</span>
                  <ExternalLink {...ICON} size={12} className="text-ink-4" />
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
                <Icon {...ICON} />
                <span className="flex-1 truncate">{item.label}</span>
                {item.id === "servers" && (
                  <span className="font-mono text-2xs text-ink-dim">{serverCount}</span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}
