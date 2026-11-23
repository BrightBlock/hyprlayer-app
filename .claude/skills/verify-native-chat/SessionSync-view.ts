interface Vars {
  next: number;
  applied: number;
  held: number[];
  detached: boolean;
}

export const title = "A desktop following a session's log";

export const state = (v: Vars) => {
  const behind = v.next - 1 - v.applied;
  const held = Object.keys(v.held ?? {}).length;
  const lag = v.detached ? ", its stream lagged" : "";
  return `${behind} rows not shown, ${held} held${lag}`;
};

export const group = (v: Vars) => (v.detached ? "lagged" : "live");

export const colors = { live: "blue", lagged: "orange" };

const labels: Record<string, string> = {
  Store: "the session stores an event",
  Forward: "the server streams the next frame",
  Receive: "the app receives a frame",
  PullRead: "you pull, and the server reads a page",
  PullApply: "the page lands",
};

export const step = (action: string) => labels[action] ?? action;
