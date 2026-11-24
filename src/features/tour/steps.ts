export type TourPath = "firstRun" | "whatsNew";

/** A real element carries `data-tour="<id>"`; ids, never class selectors, so a restyle can't break the tour. */
export type AnchorId =
  | "projects.add"
  | "lead.dock"
  | "lead.dock.input"
  | "lead.dock.remote"
  | "lead.panel.running"
  | "lead.panel.queue"
  | "lead.panel.edge"
  | "agent.tab"
  | "statusbar.server";

export type Side = "top" | "bottom" | "left" | "right";

/** What the step does to put its anchor on screen before it looks for it. */
export type Reveal = "none" | "projectBoard" | "leadExpanded" | "tourAgentTab";

/** The existing signal that completes a TRY IT step. */
export type Signal = "projectAdded" | "leadMessageSent" | "agentSpawned" | "leadPanelAdjusted" | "leadAnswered";

/** What must hold before the step can run in place. */
export type Need = "project" | "lead" | "engine" | "server" | "leadPanel";

export type TourStep = {
  id: string;
  path: TourPath;
  /** Seen features drop out of later what's-new runs. */
  feature: string;
  /** The app version the feature shipped in. */
  since: string;
  anchor: AnchorId;
  /** The step reads fine without its anchor (it explains behaviour, not a control): no "isn't on screen". */
  anchorOptional?: boolean;
  side: Side;
  reveal: Reveal;
  needs: Need[];
  title: string;
  /** `{chord:<command id>}` renders the user's binding for that command. */
  body: string;
  tryIt?: { signal: Signal; waiting: string; done: string; prompt?: string; promptLabel?: string };
};

/** The tour's first task: costs one agent's read, touches nothing in the user's checkout. */
export const SAFE_FIRST_TASK =
  "Spawn one agent to read this project and tell me in five lines what it does and how it's laid out. " +
  "Read only: no edits, commits or PRs. Archive its workstream when it reports back.";

/** The last release before the tour: an upgrade with no tour store saw this one. */
export const PRE_TOUR_VERSION = "0.8.24";
const LEAD_RELEASE = "0.8.25";

export const STEPS: TourStep[] = [
  {
    id: "fr.project",
    path: "firstRun",
    feature: "projects.open",
    since: LEAD_RELEASE,
    anchor: "projects.add",
    side: "right",
    reveal: "none",
    needs: ["server"],
    title: "Start with a project",
    body: "Pick a repository you know. The tour only reads it: nothing in your checkout changes.",
    tryIt: { signal: "projectAdded", waiting: "Waiting for a project", done: "Project added" },
  },
  {
    id: "fr.lead",
    path: "firstRun",
    feature: "lead.dock",
    since: LEAD_RELEASE,
    anchor: "lead.dock",
    side: "top",
    reveal: "projectBoard",
    needs: ["server", "project", "lead"],
    title: "Meet your Lead Engineer",
    body:
      "Every project has one, here at the bottom of the board. Tell it what you want. It doesn't write code: " +
      "it opens a workstream for each piece of work (its own worktree and branch), spawns an agent into it on " +
      "your default engine, watches it, and merges the PR once it validates.",
  },
  {
    id: "fr.prompt",
    path: "firstRun",
    feature: "lead.prompt",
    since: LEAD_RELEASE,
    anchor: "lead.dock.input",
    side: "top",
    reveal: "projectBoard",
    needs: ["server", "project", "lead", "engine"],
    title: "Give it something small",
    body: "Start with a job that only reads. The Lead makes a scratch worktree for it, so your checkout stays as it is.",
    tryIt: {
      signal: "leadMessageSent",
      waiting: "Waiting for you to press Send",
      done: "Sent. The Lead is reading the project.",
      prompt: SAFE_FIRST_TASK,
      promptLabel: "Put it in the Lead’s box",
    },
  },
  {
    id: "fr.handoff",
    path: "firstRun",
    feature: "lead.panel.running",
    since: LEAD_RELEASE,
    anchor: "lead.panel.running",
    side: "left",
    reveal: "leadExpanded",
    needs: ["server", "project", "lead", "leadPanel"],
    title: "It plans, then hands off",
    body:
      "The Lead opens a workstream and spawns an agent into it. This panel shows its Queue, what’s Running and " +
      "how far each piece has got. Drag a queued item’s handle to reorder it, or remove it to cancel it. " +
      "Drag the panel’s edge to resize it; {chord:view.toggleLeadPanel} hides it.",
    tryIt: { signal: "agentSpawned", waiting: "Waiting for the Lead to spawn an agent", done: "Agent spawned" },
  },
  {
    id: "fr.agent",
    path: "firstRun",
    feature: "agent.tab",
    since: LEAD_RELEASE,
    anchor: "agent.tab",
    side: "bottom",
    reveal: "tourAgentTab",
    needs: ["server", "project"],
    title: "Agents work in their own tabs",
    body:
      "Open one to watch it. It runs on your default engine and in your permission mode, bypass included. " +
      "Messages from the Lead are marked “from Lead Engineer”. If the agent has a question, it goes to the " +
      "Lead, and the Lead brings it to you.",
  },
  {
    id: "fr.report",
    path: "firstRun",
    feature: "lead.report",
    since: LEAD_RELEASE,
    anchor: "lead.dock",
    side: "top",
    reveal: "projectBoard",
    needs: ["server", "project", "lead"],
    title: "It reports back, and asks",
    body:
      "When the agent finishes, the Lead reads its result and answers here. It asks before it archives a " +
      "workstream, unless you run in bypass mode. Next time, ask for a real change: it’ll open a PR and merge " +
      "it once it validates.",
    tryIt: { signal: "leadAnswered", waiting: "Answer the Lead’s question", done: "The workstream is archived" },
  },
  {
    id: "wn.panel",
    path: "whatsNew",
    feature: "lead.panel.resize",
    since: LEAD_RELEASE,
    anchor: "lead.panel.edge",
    side: "left",
    reveal: "leadExpanded",
    needs: ["server", "project", "lead", "leadPanel"],
    title: "Resize the Lead panel, or hide it",
    body:
      "Drag this edge to give the chat more room. {chord:view.toggleLeadPanel} or the panel button in the top " +
      "bar hides the panel and brings it back. It now ends with live host meters.",
    tryIt: { signal: "leadPanelAdjusted", waiting: "Press the shortcut, or drag the edge", done: "Panel adjusted" },
  },
  {
    id: "wn.queue",
    path: "whatsNew",
    feature: "lead.queue",
    since: LEAD_RELEASE,
    anchor: "lead.panel.queue",
    side: "left",
    reveal: "leadExpanded",
    needs: ["server", "project", "lead", "leadPanel"],
    title: "The queue is yours to edit",
    body:
      "Planned work the Lead hasn’t started waits here. Drag an item’s handle, or focus it and press ↑ and ↓, " +
      "to change the order. Remove an item and it’s cancelled: the Lead checks the queue again right before it " +
      "starts anything.",
  },
  {
    id: "wn.remote",
    path: "whatsNew",
    feature: "lead.remoteControl",
    since: LEAD_RELEASE,
    anchor: "lead.dock.remote",
    side: "top",
    reveal: "projectBoard",
    needs: ["server", "project", "lead"],
    title: "Reach the Lead from your phone",
    body:
      "Turn on Remote Control and this Lead shows up in the Claude app. Open in Claude and Copy link appear " +
      "here once the Lead is running.",
  },
  {
    id: "wn.resume",
    path: "whatsNew",
    feature: "agents.resume",
    since: LEAD_RELEASE,
    anchor: "agent.tab",
    anchorOptional: true,
    side: "bottom",
    reveal: "none",
    needs: ["server"],
    title: "Agents pick up where they left off",
    body:
      "If the server restarts, a terminal agent that was mid-turn resumes in its own tab and conversation. Only " +
      "the tab you’re looking at resizes an agent’s terminal, so a second window can’t squash it.",
  },
  {
    id: "wn.server",
    path: "whatsNew",
    feature: "server.button",
    since: LEAD_RELEASE,
    anchor: "statusbar.server",
    side: "top",
    reveal: "none",
    needs: [],
    title: "Your server, at a glance",
    body:
      "The app is a client of a hyprlayer-server, on this machine or over ssh. The Server button shows that " +
      "host’s CPU, memory and running agents; click it for detail or to switch servers. Turn the stats off in " +
      "quick settings.",
  },
];

/** The card's one-liners per step, and its "Also" line per version: no step, no anchor. */
export const CARD_LINES: Record<string, string> = {
  "wn.panel": "Queue, Running, Stats and host meters, beside the Lead’s chat.",
  "wn.queue": "Drag to reorder planned work; remove an item to cancel it.",
  "wn.remote": "Reach the Lead from the Claude app on your phone.",
  "wn.resume": "In their own tabs, mid-conversation.",
  "wn.server": "Host stats in the status bar; switch servers from the same menu.",
};

export const CARD_TITLES: Record<string, string> = {
  "wn.panel": "A Lead panel you can resize and hide",
  "wn.queue": "Reorder or cancel queued work",
  "wn.remote": "Remote Control",
  "wn.resume": "Agents resume after a server restart",
  "wn.server": "Host stats in the Server button",
};

export const ALSO_NEW: Record<string, string> = {
  [LEAD_RELEASE]:
    "OpenCode 2.4, with sign-in under Settings → OpenCode. Long chats open faster, and busy indicators hold " +
    "still when the GPU can’t composite.",
};
