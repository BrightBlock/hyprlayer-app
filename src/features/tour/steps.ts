export type TourPath = "firstRun" | "whatsNew";

/** A real element carries `data-tour="<id>"`; ids, never class selectors, so a restyle can't break the tour. */
export type AnchorId =
  | "projects.add"
  | "sidebar.project"
  | "lead.dock"
  | "lead.dock.header"
  | "lead.dock.expand"
  | "lead.dock.input"
  | "lead.dock.remote"
  | "lead.panel.running"
  | "lead.panel.queue"
  | "lead.panel.host"
  | "lead.panel.edge"
  | "agent.tab"
  | "statusbar.server";

export type Side = "top" | "bottom" | "left" | "right";

/** What the step does to put its anchor on screen before it looks for it. */
export type Reveal = "none" | "projectBoard" | "leadExpanded" | "tourAgentTab";

/** The existing signal that completes a TRY IT step. */
export type Signal =
  | "projectAdded"
  | "leadOpened"
  | "leadMessageSent"
  | "leadExpanded"
  | "agentSpawned"
  | "leadPanelAdjusted"
  | "leadAnswered";

/** What must hold before the step can run in place. */
export type Need = "project" | "lead" | "engine" | "server" | "leadPanel";

export type TourStep = {
  id: string;
  path: TourPath;
  /** Seen features drop out of later what's-new runs. */
  feature: string;
  /** The app version the feature shipped in. */
  since: string;
  /** Null: an intro card, shown in the middle of the window over a scrim. */
  anchor: AnchorId | null;
  /** A second control the step names, outlined but not pointed at. Left out when it isn't on screen. */
  also?: { anchor: AnchorId; label: string };
  /** The step reads fine without its anchor (it explains behaviour, not a control): no "isn't on screen". */
  anchorOptional?: boolean;
  side: Side;
  reveal: Reveal;
  needs: Need[];
  title: string;
  /** Paragraphs split on a blank line. `{chord:<command id>}` renders the user's binding for that command. */
  body: string;
  /** A numbered list under the body. */
  points?: { title: string; text: string }[];
  /** A dimmer last line. */
  note?: string;
  tryIt?: { signal: Signal; waiting: string; done: string; prompt?: string; promptLabel?: string };
};

/** The tour's first task: costs one agent's read, touches nothing in the user's checkout. */
export const SAFE_FIRST_TASK =
  "Spawn one agent to read this project and tell me in five lines what it does and how it's laid out. " +
  "Read only: no edits, commits or PRs. Archive its workstream when it reports back.";

/** The last release before the tour: an upgrade with no tour store saw this one. */
export const PRE_TOUR_VERSION = "0.8.24";
const LEAD_RELEASE = "0.8.25";

const FIRST_RUN_TAIL = "Take the tour again any time from Help in quick settings.";

export const STEPS: TourStep[] = [
  {
    id: "fr.intro",
    path: "firstRun",
    feature: "lead.intro",
    since: LEAD_RELEASE,
    anchor: null,
    side: "bottom",
    reveal: "none",
    needs: [],
    title: "Meet your Lead Engineer",
    body:
      "Every project gets a Lead Engineer: an engineering lead you talk to. Tell it what you want, and it gets " +
      "the work done through other agents.\n\nIt never writes code itself. It plans the work, starts an agent for " +
      "each piece, keeps an eye on them, and comes to you when it needs a decision.",
    note: "If you’ve only run one agent at a time before, you’ll now ask for outcomes and leave the agents to the Lead.",
  },
  {
    id: "fr.how",
    path: "firstRun",
    feature: "lead.how",
    since: LEAD_RELEASE,
    anchor: null,
    side: "bottom",
    reveal: "none",
    needs: [],
    title: "What the Lead does for you",
    body: "",
    points: [
      { title: "Plans", text: "Breaks your request into pieces. Work it hasn’t started waits in a queue you can reorder." },
      {
        title: "Starts the work",
        text:
          "Gives each piece a workstream, with its own worktree and branch, and spawns an agent there with a " +
          "written brief, on your default engine.",
      },
      { title: "Watches", text: "Follows every agent. When one has a question, the Lead brings it to you." },
      { title: "Lands it", text: "Checks the result, merges the PR once it validates, and archives the workstream." },
    ],
    note:
      "Next you’ll open the Lead, give it one small read-only job, and watch it start an agent, follow it and " +
      "report back. Nothing in your checkout changes. It takes about 3 minutes.",
  },
  {
    id: "fr.open",
    path: "firstRun",
    feature: "lead.open",
    since: LEAD_RELEASE,
    anchor: "sidebar.project",
    side: "right",
    reveal: "none",
    needs: ["server", "project", "lead"],
    title: "Open your project’s Lead",
    body:
      "Each project in the sidebar has its own Lead. Click the project’s name to open its board: the Lead’s dock " +
      "sits at the bottom. From anywhere in the project, {chord:lead.open} opens the Lead’s chat.\n\nThe " +
      "project’s icon glows amber when the Lead, or one of its agents, needs you.",
    tryIt: { signal: "leadOpened", waiting: "Click the project in the sidebar", done: "The Lead’s dock is open" },
  },
  {
    id: "fr.prompt",
    path: "firstRun",
    feature: "lead.prompt",
    since: LEAD_RELEASE,
    anchor: "lead.dock",
    also: { anchor: "lead.dock.header", label: "Click the title line or the arrow to expand" },
    side: "top",
    reveal: "projectBoard",
    needs: ["server", "project", "lead", "engine"],
    title: "Talk to the Lead here",
    body:
      "This bar at the bottom of the board is the Lead’s dock. Ask it for work in plain words, and it replies " +
      "here.\n\nTo see the whole conversation, click the dock’s title line or the arrow at its right end. That " +
      "opens the full chat.\n\nStart with a job that only reads. The Lead gives it a scratch worktree, so your " +
      "checkout stays as it is.",
    tryIt: {
      signal: "leadMessageSent",
      waiting: "Waiting for you to press Send",
      done: "Sent. The Lead is planning.",
      prompt: SAFE_FIRST_TASK,
      promptLabel: "Put it in the Lead’s box",
    },
  },
  {
    id: "fr.expand",
    path: "firstRun",
    feature: "lead.expand",
    since: LEAD_RELEASE,
    anchor: "lead.dock.expand",
    side: "top",
    reveal: "projectBoard",
    needs: ["server", "project", "lead"],
    title: "Open the full chat",
    body:
      "The dock shows the Lead’s latest line. Click that line, or the arrow, to open the whole conversation with " +
      "the Lead panel beside it.\n\nEsc shrinks it back to the dock.",
    tryIt: { signal: "leadExpanded", waiting: "Waiting for you to open the chat", done: "The full chat is open" },
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
      "The Lead opens a workstream and spawns an agent into it. Running shows each piece and how far it has got: " +
      "plan, build, validate, merged.\n\nPlanned work waits in Queue. Drag an item, or focus it and press ↑ or ↓, " +
      "to reorder it; remove it to cancel. Drag the panel’s edge to resize it; {chord:view.toggleLeadPanel} hides it.",
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
    title: "Each agent works in its own tab",
    body:
      "Open it to watch. The first message is the Lead’s brief, marked “from Lead Engineer”: the job, the limits, " +
      "and how to report back.\n\nThe agent runs on your default engine, in your permission mode. When it has a " +
      "question, it asks the Lead, and the Lead asks you.",
  },
  {
    id: "fr.stats",
    path: "firstRun",
    feature: "lead.panel.host",
    since: LEAD_RELEASE,
    anchor: "lead.panel.host",
    also: { anchor: "statusbar.server", label: "The same numbers, a click away" },
    side: "left",
    reveal: "leadExpanded",
    needs: ["server", "project", "lead", "leadPanel"],
    title: "See what the server is carrying",
    body:
      "Every agent runs on one server. Stats counts live agents against the Lead’s limit, with workstreams and " +
      "open PRs.\n\nHost shows the server itself. CPU is how busy it is right now. Memory is what’s in use, with " +
      "cache in the paler part. Swap climbs when memory runs out. When memory stays full or swap climbs, start " +
      "fewer agents.\n\nClick the Server button in the status bar for the same numbers. To keep them on the button, " +
      "turn on Show server stats in status bar in quick settings.",
  },
  {
    id: "fr.report",
    path: "firstRun",
    feature: "lead.report",
    since: LEAD_RELEASE,
    anchor: "lead.panel.running",
    side: "left",
    reveal: "leadExpanded",
    needs: ["server", "project", "lead", "leadPanel"],
    title: "It reports back, then tidies up",
    body:
      "When the agent finishes, the Lead reads its result and sums it up in the chat. Then it archives the " +
      "workstream, which leaves Running. It asks first, unless you run in bypass mode.\n\nNext time, ask for a " +
      "real change. The Lead opens a PR for it and merges it once it validates.",
    note: FIRST_RUN_TAIL,
    tryIt: {
      signal: "leadAnswered",
      waiting: "Answer the Lead’s question in the chat",
      done: "The workstream is archived",
    },
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
      "The app is a client of a hyprlayer-server, on this machine or over ssh. Click the Server button for that " +
      "host’s CPU, memory and running agents, or to switch servers. Turn on Show server stats in status bar, in " +
      "quick settings, to keep them on the button.",
  },
];

/** The what's-new card's lead-in per version: what a user upgrading from it has never seen. */
export const CARD_INTRO: Record<string, { title: string; body: string; note: string }> = {
  [LEAD_RELEASE]: {
    title: "Meet the Lead Engineer",
    body:
      "Every project now has a Lead Engineer: an engineering lead you talk to. Say what you want. It plans the " +
      "work, starts agents in their own workstreams, brings you their questions, and merges what validates. It " +
      "never writes code itself.",
    note: "One small read-only job. Nothing in your checkout changes.",
  },
};

/** The card's one-liners per step, and its "Also" line per version: no step, no anchor. */
export const CARD_LINES: Record<string, string> = {
  "wn.remote": "Reach the Lead from the Claude app on your phone.",
  "wn.resume": "In their own tabs, mid-conversation.",
  "wn.server": "CPU, memory and agents for the server you’re on.",
};

export const CARD_TITLES: Record<string, string> = {
  "wn.remote": "Remote Control",
  "wn.resume": "Agents resume after a restart",
  "wn.server": "Server stats in the status bar",
};

export const ALSO_NEW: Record<string, string> = {
  [LEAD_RELEASE]:
    "OpenCode 2.4, with sign-in under Settings → OpenCode. Long chats open faster, and busy indicators hold " +
    "still when the GPU can’t composite.",
};
