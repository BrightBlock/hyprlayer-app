/**
 * The keyboard-command catalog — the single source of truth for every bindable
 * operation, its per-OS default chord, and the context in which it's available.
 *
 * Defaults live here (not in the backend) so they can evolve per-OS without a
 * settings migration: a user who hasn't overridden a command always tracks the
 * catalog. Chords use the abstract `mod` modifier, which formats and matches as
 * ⌘ on macOS and Ctrl elsewhere — so the per-OS pair is usually identical, but
 * the structure still lets a single command diverge a default later.
 */

import { normalizeChord, type Chord } from "./chord";

export type CommandCategory =
  | "Navigation"
  | "View"
  | "Workspace"
  | "Terminal"
  | "Session"
  | "Library";

/** Where a command is available — informational, and drives UI grouping. It is
 *  NOT the runtime gate: whether a handler is registered (see `registry.ts`) is
 *  the real context predicate. */
export type CommandWhen = "always" | "project" | "workstream" | "session";

export type CommandDef = {
  /** Stable id, persisted in settings overrides — never rename. */
  id: string;
  label: string;
  category: CommandCategory;
  /** Default chord per OS; null = no default binding. */
  defaults: { mac: Chord | null; other: Chord | null };
  /** Extra chords that also fire this command, for actions the platform spells
   *  two ways (Safari cycles tabs with both ⌃⇥ and ⌘⇧]). Not rebindable on their
   *  own. They belong to the default binding, so an override drops them: Settings
   *  has no row to show them on. A primary binding beats an alternate on
   *  collision. */
  alternates?: { mac: Chord[]; other: Chord[] };
  /** Where the command is available (informational + drives UI grouping). */
  when: CommandWhen;
};

/** How many tabs `tab.select1`…`tab.selectN` address directly. Safari stops at
 *  eight and gives the ninth chord to "last tab", however many tabs there are. */
export const TAB_SELECT_COUNT = 8;

/** Command id selecting the `index`-th tab (0-based) of the focused pane. */
export const tabSelectId = (index: number): string => `tab.select${index + 1}`;

export const COMMANDS: CommandDef[] = [
  // Navigation
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
    // ⌘1–⌘3 belong to the tabs (Safari's jump-to-tab), so app navigation sits a
    // modifier over. On macOS that's the *literal* Control key; elsewhere physical
    // Ctrl maps to `mod`, which the tabs use, so those platforms take Alt. ⌃1–⌃3
    // also switch Spaces on macOS when several desktops exist: the OS wins, and
    // the user can rebind.
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
  // View
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
    label: "Toggle Archived Agents",
    category: "View",
    when: "workstream",
    defaults: { mac: "mod+shift+a", other: "mod+shift+a" },
  },
  {
    id: "view.toggleMarkdownPreview",
    label: "Toggle Markdown Preview",
    category: "View",
    when: "workstream",
    // Only registered while a markdown file is open in the file viewer (see
    // FileViewerTab) — inert everywhere else via the registry's context gate.
    defaults: { mac: "mod+shift+v", other: "mod+shift+v" },
  },
  // Workspace
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
    // ⌘W, as in Safari, and the same action as the tab's × button: an agent is
    // archived (conversation, branch and worktree intact, restorable with ⌘⇧T),
    // a terminal or viewer closes. The destructive close is one modifier away on
    // `tab.close`. ⌘W is free only because the macOS menu omits Close Window
    // (see src-tauri/src/lib.rs).
    defaults: { mac: "mod+w", other: "mod+w" },
  },
  {
    id: "tab.close",
    label: "Close Agent Tab & End Session",
    category: "Workspace",
    when: "project",
    // Irreversible: kills the session and drops the transcript.
    defaults: { mac: "mod+shift+w", other: "mod+shift+w" },
  },
  {
    id: "tab.reopen",
    label: "Reopen Last Closed Agent Tab",
    category: "Workspace",
    when: "project",
    // Safari's ⌘⇧T. Restores the most recently archived agent, so it undoes a ⌘W
    // (or an × click) but not a ⌘⇧W, which ended the session for good. Inert with
    // an empty archive — no handler is registered, so the keystroke passes through.
    defaults: { mac: "mod+shift+t", other: "mod+shift+t" },
  },
  {
    id: "tab.toggleSplit",
    label: "Toggle Split Pane",
    category: "Workspace",
    when: "project",
    defaults: { mac: "mod+backslash", other: "mod+backslash" },
  },
  {
    id: "tab.next",
    label: "Next Agent Tab",
    category: "Workspace",
    when: "project",
    // Cycle agent tabs the way every browser/editor does: the *literal* Control
    // key, not the `mod` abstraction. On macOS that's `ctrl+tab` (⌘Tab is the OS
    // app switcher — reserved). Elsewhere physical Ctrl maps to `mod`, so Ctrl+Tab
    // would be `mod+tab` — also reserved — so the non-mac default uses the other
    // standard tab-cycle chord, Ctrl+PageDown.
    defaults: { mac: "ctrl+tab", other: "mod+pagedown" },
    // Safari's second spelling. Only matchable because `keyToken` folds the
    // shifted `}` back onto `bracketright`.
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
    // Safari's ⌘1–⌘8: the n-th tab of the focused pane, a no-op when there is no
    // n-th tab. App navigation lives on ⌃1–⌃3.
    defaults: { mac: `mod+${i + 1}`, other: `mod+${i + 1}` },
  })),
  {
    id: "tab.selectLast",
    label: "Select Last Agent Tab",
    category: "Workspace",
    when: "project",
    // Safari's ⌘9 always means "rightmost", never "the ninth".
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
    id: "ws.cycleStage",
    label: "Cycle Agent Stage",
    category: "Workspace",
    when: "workstream",
    // Steps the focused agent through its pipeline stages (General → Research →
    // Plan → Implement → Validate, wrapping) before it starts; once the session
    // is running the stage is fixed at spawn, so it's inert. Bare Shift+Tab has
    // no primary modifier, so the composer textarea handles it directly (the
    // global dispatcher's editable-target guard ignores it there) — see
    // SessionChat's onKeyDown.
    defaults: { mac: "shift+tab", other: "shift+tab" },
  },
  // Terminal
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
    // Shift+punctuation can't be a binding here (the dispatcher matches on
    // e.key, so Shift+` is "~"); a shift+letter chord round-trips cleanly.
    defaults: { mac: "mod+shift+f", other: "mod+shift+f" },
  },
  {
    id: "terminal.zoomIn",
    label: "Terminal Zoom In",
    category: "Terminal",
    when: "project",
    defaults: { mac: "mod+equal", other: "mod+equal" },
  },
  {
    id: "terminal.zoomOut",
    label: "Terminal Zoom Out",
    category: "Terminal",
    when: "project",
    defaults: { mac: "mod+minus", other: "mod+minus" },
  },
  {
    id: "terminal.zoomReset",
    label: "Terminal Zoom Reset",
    category: "Terminal",
    when: "project",
    defaults: { mac: "mod+0", other: "mod+0" },
  },
  {
    id: "scripts.run",
    label: "Run / Stop Workstream Run",
    category: "Terminal",
    when: "workstream",
    // Runs the project's Run script (stopping a prior one), or stops it if live.
    defaults: { mac: "mod+r", other: "mod+r" },
  },
  // Session
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
    // ⌘⇧T belongs to Reopen Last Closed Agent Tab, per Safari. ⌘⇧J sits beside
    // the other session command, Clear Context (⌘⇧K).
    defaults: { mac: "mod+shift+j", other: "mod+shift+j" },
  },
  {
    id: "session.historyPrev",
    label: "Recall Previous Prompt",
    category: "Session",
    when: "session",
    // Arrow combos need a primary modifier to fire while the composer textarea
    // is focused (see useKeybindings' editable-target guard); Alt+↑/↓ are free,
    // non-reserved, and read as the classic "step through history" gesture.
    defaults: { mac: "alt+up", other: "alt+up" },
  },
  {
    id: "session.historyNext",
    label: "Recall Next Prompt",
    category: "Session",
    when: "session",
    defaults: { mac: "alt+down", other: "alt+down" },
  },
  // Library
  {
    id: "doc.new",
    label: "New Document",
    category: "Library",
    when: "always",
    defaults: { mac: "mod+n", other: "mod+n" },
  },
];

/** Categories in display order (matches the catalog's grouping above). */
export const CATEGORY_ORDER: CommandCategory[] = [
  "Navigation",
  "View",
  "Workspace",
  "Terminal",
  "Session",
  "Library",
];

/** Commands grouped by category in display order, with empty groups dropped.
 *  The catalog owns the grouping so the UI renders from it directly instead of
 *  re-scanning and re-filtering the flat list on every render. */
export const COMMANDS_BY_CATEGORY: ReadonlyArray<{
  category: CommandCategory;
  commands: CommandDef[];
}> = CATEGORY_ORDER.map((category) => ({
  category,
  commands: COMMANDS.filter((c) => c.category === category),
})).filter((group) => group.commands.length > 0);

/** The catalog default chord for a command on this OS, normalized; null = none. */
export function defaultChord(cmd: CommandDef, isMac: boolean): Chord | null {
  const raw = isMac ? cmd.defaults.mac : cmd.defaults.other;
  return raw ? normalizeChord(raw) : null;
}

/** The command's alternate chords on this OS, normalized (empty when it has none). */
export function alternateChords(cmd: CommandDef, isMac: boolean): Chord[] {
  const raw = (isMac ? cmd.alternates?.mac : cmd.alternates?.other) ?? [];
  return raw.map(normalizeChord);
}

/** Lookup a command definition by id. */
export function commandById(id: string): CommandDef | undefined {
  return COMMANDS.find((c) => c.id === id);
}
