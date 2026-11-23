interface Vars {
  alive: boolean;
  waker: string;
  wsArchived: boolean;
  received: number;
}

export const title = "Waking a session after server restarts";

export const state = (v: Vars) => {
  const harness = v.alive ? "claude running" : "no claude";
  const waker = v.waker === "due" ? ", a wake is due" : "";
  const ws = v.wsArchived ? ", workstream archived" : "";
  return `${harness}${waker}${ws}, ${v.received} unanswered wakes`;
};

export const group = (v: Vars) => (v.wsArchived ? "archived" : "active");

export const colors = { active: "blue", archived: "orange" };

const labels: Record<string, string> = {
  Restart: "the server restarts",
  WakerCheck: "the waker checks the session",
  WakerWake: "the waker resumes it and sends the wake turn",
  StoreEcho: "the wake turn is stored",
  ClaudeFinishes: "claude finishes a turn",
  Archive: "you archive the workstream",
};

export const step = (action: string) => labels[action] ?? action;
