interface Vars {
  procs: number;
  row: string;
  working: boolean;
  resumes: number;
  archived: boolean;
  lock: string;
}

export const title = "Resuming a PTY agent after server restarts";

export const state = (v: Vars) => {
  const live = v.procs === 0 ? "no process" : v.procs === 1 ? "one process" : `${v.procs} processes`;
  const row = v.row === "stored" ? (v.working ? "stored mid-turn" : "stored idle") : "no row";
  const lock = v.lock === "none" ? "" : `, ${v.lock} holds the lock`;
  const ws = v.archived ? ", workstream archived" : "";
  return `${live}, ${row}, ${v.resumes} resumes${lock}${ws}`;
};

export const group = (v: Vars) => (v.archived ? "archived" : "active");

export const colors = { active: "blue", archived: "orange" };

const labels: Record<string, string> = {
  StartTurn: "the agent starts a turn",
  FinishTurn: "the agent finishes its turn",
  ExitsOnItsOwn: "the agent exits while the server runs",
  Restart: "the server restarts",
  PassCheck: "the startup pass looks at the row",
  PassLock: "the startup pass takes the terminal's lock",
  PassRecheck: "the startup pass checks again under the lock",
  PassLaunch: "the startup pass resumes it in place",
  MsgWant: "its spawner messages the exited agent",
  MsgLock: "the message takes the terminal's lock",
  MsgRecheck: "the message checks again under the lock",
  MsgLaunch: "the message resumes it in place",
  DeskWant: "a desktop relaunches the conversation",
  DeskLock: "the desktop's launch takes the terminal's lock",
  DeskEnd: "the desktop's launch ends the stored agent",
  DeskLaunch: "the desktop launches its own agent",
  Close: "you close the tab",
  Archive: "you archive the workstream",
};

export const step = (action: string) => labels[action] ?? action;
