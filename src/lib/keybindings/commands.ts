
import { normalizeChord, type Chord } from "./chord";

type CommandCategory =
  | "Navigation"
  | "View"
  | "Workspace"
  | "Terminal"
  | "Session"
  | "Library";

export type CommandWhen = "always" | "project" | "workstream" | "session";

export type CommandDef = {
  id: string;
  label: string;
  category: CommandCategory;
  defaults: { mac: Chord | null; other: Chord | null };
  alternates?: { mac: Chord[]; other: Chord[] };
  when: CommandWhen;
};

export const TAB_SELECT_COUNT = 8;

export const tabSelectId = (index: number): string => `tab.select${index + 1}`;

export const COMMANDS: CommandDef[] = [
  {
    id: "search.open",
    label: "Search Everything",
    category: "Navigation",
    when: "always",
    defaults: { mac: "mod+p", other: "mod+p" },
  },
  {
    id: "prompts.palette",
    label: "Prompt Library",
    category: "Navigation",
    when: "always",
    defaults: { mac: "mod+k", other: "mod+k" },
  },
  {
    id: "nav.dashboard",
    label: "Go to Dashboard",
    category: "Navigation",
    when: "always",
    defaults: { mac: "ctrl+1", other: "alt+1" },
  },
  {
    id: "nav.library",
    label: "Go to Library",
    category: "Navigation",
    when: "always",
    defaults: { mac: "ctrl+2", other: "alt+2" },
  },
  {
    id: "nav.settings",
    label: "Open Settings",
    category: "Navigation",
    when: "always",
    defaults: { mac: "mod+comma", other: "mod+comma" },
  },
  {
    id: "nav.projectDocs",
    label: "Toggle Project Docs",
    category: "Navigation",
    when: "project",
    defaults: { mac: "ctrl+3", other: "alt+3" },
  },
  {
    id: "nav.back",
    label: "Go Back",
    category: "Navigation",
    when: "always",
    defaults: { mac: "mod+bracketleft", other: "mod+bracketleft" },
  },
  {
    id: "view.toggleProjectSidebar",
    label: "Toggle Project Sidebar",
    category: "View",
    when: "always",
    defaults: { mac: "mod+b", other: "mod+b" },
  },
  {
    id: "view.toggleChangesSidebar",
    label: "Toggle Changes Sidebar",
    category: "View",
    when: "project",
    defaults: { mac: "mod+shift+b", other: "mod+shift+b" },
  },
  {
    id: "view.toggleArchivedAgents",
    label: "Toggle Archived Sessions",
    category: "View",
    when: "workstream",
    defaults: { mac: "mod+shift+a", other: "mod+shift+a" },
  },
  {
    id: "view.toggleMarkdownPreview",
    label: "Toggle Markdown Preview",
    category: "View",
    when: "workstream",
    defaults: { mac: "mod+shift+v", other: "mod+shift+v" },
  },
  {
    id: "view.zoomIn",
    label: "Zoom In",
    category: "View",
    when: "always",
    defaults: { mac: "mod+equal", other: "mod+equal" },
  },
  {
    id: "view.zoomOut",
    label: "Zoom Out",
    category: "View",
    when: "always",
    defaults: { mac: "mod+minus", other: "mod+minus" },
  },
  {
    id: "view.zoomReset",
    label: "Reset Zoom",
    category: "View",
    when: "always",
    defaults: { mac: "mod+0", other: "mod+0" },
  },
  {
    id: "project.open",
    label: "Open Project",
    category: "Workspace",
    when: "always",
    defaults: { mac: "mod+o", other: "mod+o" },
  },
  {
    id: "tab.new",
    label: "New Agent Tab",
    category: "Workspace",
    when: "project",
    defaults: { mac: "mod+t", other: "mod+t" },
  },
  {
    id: "tab.archive",
    label: "Close Agent Tab",
    category: "Workspace",
    when: "project",
    defaults: { mac: "mod+w", other: "mod+w" },
  },
  {
    id: "tab.close",
    label: "Close Agent Tab & End Session",
    category: "Workspace",
    when: "project",
    defaults: { mac: "mod+shift+w", other: "mod+shift+w" },
  },
  {
    id: "tab.reopen",
    label: "Reopen Last Closed Agent Tab",
    category: "Workspace",
    when: "project",
    defaults: { mac: "mod+shift+t", other: "mod+shift+t" },
  },
  {
    id: "tab.toggleSplit",
    label: "Split Pane Right",
    category: "Workspace",
    when: "project",
    defaults: { mac: "mod+backslash", other: "mod+backslash" },
  },
  {
    id: "pane.splitDown",
    label: "Split Pane Down",
    category: "Workspace",
    when: "project",
    defaults: { mac: "mod+shift+backslash", other: "mod+shift+backslash" },
  },
  {
    id: "pane.close",
    label: "Close Pane (Keep Tabs)",
    category: "Workspace",
    when: "project",
    defaults: { mac: null, other: null },
  },
  {
    id: "pane.focusLeft",
    label: "Focus Pane Left",
    category: "Workspace",
    when: "project",
    defaults: { mac: "mod+alt+left", other: "mod+alt+left" },
  },
  {
    id: "pane.focusRight",
    label: "Focus Pane Right",
    category: "Workspace",
    when: "project",
    defaults: { mac: "mod+alt+right", other: "mod+alt+right" },
  },
  {
    id: "pane.focusUp",
    label: "Focus Pane Up",
    category: "Workspace",
    when: "project",
    defaults: { mac: "mod+alt+up", other: "mod+alt+up" },
  },
  {
    id: "pane.focusDown",
    label: "Focus Pane Down",
    category: "Workspace",
    when: "project",
    defaults: { mac: "mod+alt+down", other: "mod+alt+down" },
  },
  {
    id: "tab.next",
    label: "Next Agent Tab",
    category: "Workspace",
    when: "project",
    defaults: { mac: "ctrl+tab", other: "mod+pagedown" },
    alternates: { mac: ["mod+shift+bracketright"], other: ["mod+shift+bracketright"] },
  },
  {
    id: "tab.prev",
    label: "Previous Agent Tab",
    category: "Workspace",
    when: "project",
    defaults: { mac: "ctrl+shift+tab", other: "mod+pageup" },
    alternates: { mac: ["mod+shift+bracketleft"], other: ["mod+shift+bracketleft"] },
  },
  ...Array.from({ length: TAB_SELECT_COUNT }, (_, i): CommandDef => ({
    id: tabSelectId(i),
    label: `Select Agent Tab ${i + 1}`,
    category: "Workspace",
    when: "project",
    defaults: { mac: `mod+${i + 1}`, other: `mod+${i + 1}` },
  })),
  {
    id: "tab.selectLast",
    label: "Select Last Agent Tab",
    category: "Workspace",
    when: "project",
    defaults: { mac: "mod+9", other: "mod+9" },
  },
  {
    id: "ws.new",
    label: "New Workstream",
    category: "Workspace",
    when: "project",
    defaults: { mac: "mod+shift+n", other: "mod+shift+n" },
  },
  {
    id: "ws.commit",
    label: "Commit",
    category: "Workspace",
    when: "workstream",
    defaults: { mac: "mod+shift+enter", other: "mod+shift+enter" },
  },
  {
    id: "ws.ship",
    label: "Ship (open PR)",
    category: "Workspace",
    when: "workstream",
    defaults: { mac: "mod+shift+p", other: "mod+shift+p" },
  },
  {
    id: "ws.review",
    label: "Review Changes",
    category: "Workspace",
    when: "workstream",
    defaults: { mac: "mod+shift+r", other: "mod+shift+r" },
  },
  {
    id: "ws.cycleStage",
    label: "Cycle Agent Stage",
    category: "Workspace",
    when: "workstream",
    defaults: { mac: "shift+tab", other: "shift+tab" },
  },
  {
    id: "terminal.open",
    label: "Toggle Terminal & Scripts Dock",
    category: "Terminal",
    when: "workstream",
    defaults: { mac: "mod+backquote", other: "mod+backquote" },
  },
  {
    id: "terminal.toggleFullscreen",
    label: "Toggle Dock Full Height",
    category: "Terminal",
    when: "workstream",
    defaults: { mac: "mod+shift+f", other: "mod+shift+f" },
  },
  {
    id: "terminal.zoomIn",
    label: "Increase Terminal Font Size",
    category: "Terminal",
    when: "project",
    defaults: { mac: "mod+shift+equal", other: "mod+shift+equal" },
  },
  {
    id: "terminal.zoomOut",
    label: "Decrease Terminal Font Size",
    category: "Terminal",
    when: "project",
    defaults: { mac: "mod+shift+minus", other: "mod+shift+minus" },
  },
  {
    id: "terminal.zoomReset",
    label: "Reset Terminal Font Size",
    category: "Terminal",
    when: "project",
    defaults: { mac: "mod+shift+0", other: "mod+shift+0" },
  },
  {
    id: "scripts.run",
    label: "Run / Stop Workstream Run",
    category: "Terminal",
    when: "workstream",
    defaults: { mac: "mod+r", other: "mod+r" },
  },
  {
    id: "session.stop",
    label: "Stop / Interrupt Turn",
    category: "Session",
    when: "session",
    defaults: { mac: "mod+period", other: "mod+period" },
  },
  {
    id: "session.clear",
    label: "Clear Context (end session)",
    category: "Session",
    when: "session",
    defaults: { mac: "mod+shift+k", other: "mod+shift+k" },
  },
  {
    id: "session.togglePlan",
    label: "Show / Hide Tasks",
    category: "Session",
    when: "session",
    defaults: { mac: "mod+shift+j", other: "mod+shift+j" },
  },
  {
    id: "session.historyPrev",
    label: "Recall Previous Prompt",
    category: "Session",
    when: "session",
    defaults: { mac: "alt+up", other: "alt+up" },
  },
  {
    id: "session.historyNext",
    label: "Recall Next Prompt",
    category: "Session",
    when: "session",
    defaults: { mac: "alt+down", other: "alt+down" },
  },
  {
    id: "doc.new",
    label: "New Document",
    category: "Library",
    when: "always",
    defaults: { mac: "mod+n", other: "mod+n" },
  },
];

const CATEGORY_ORDER: CommandCategory[] = [
  "Navigation",
  "View",
  "Workspace",
  "Terminal",
  "Session",
  "Library",
];

export const COMMANDS_BY_CATEGORY: ReadonlyArray<{
  category: CommandCategory;
  commands: CommandDef[];
}> = CATEGORY_ORDER.map((category) => ({
  category,
  commands: COMMANDS.filter((c) => c.category === category),
})).filter((group) => group.commands.length > 0);

export function defaultChord(cmd: CommandDef, isMac: boolean): Chord | null {
  const raw = isMac ? cmd.defaults.mac : cmd.defaults.other;
  return raw ? normalizeChord(raw) : null;
}

export function alternateChords(cmd: CommandDef, isMac: boolean): Chord[] {
  const raw = (isMac ? cmd.alternates?.mac : cmd.alternates?.other) ?? [];
  return raw.map(normalizeChord);
}

export function commandById(id: string): CommandDef | undefined {
  return COMMANDS.find((c) => c.id === id);
}
