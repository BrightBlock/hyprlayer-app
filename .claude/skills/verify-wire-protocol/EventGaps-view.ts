interface Vars {
  fc: boolean;
  next: number;
  cnext: number;
  missed: number;
  lostAtServer: number[];
}

export const title = "Runtime events and the gaps the client counts";

export const state = (v: Vars) => {
  const marker = v.fc ? "gap markers on" : "gap markers off";
  const lost = v.lostAtServer.length;
  const behind = v.next > v.cnext ? `, ${v.next - v.cnext} not yet read` : "";
  return `${marker}: ${lost} lost at the server, ${v.missed} counted${behind}`;
};

export const group = (v: Vars) => (v.fc ? "markers" : "nomarkers");

export const colors = { markers: "blue", nomarkers: "orange" };

const labels: Record<string, string> = {
  SrvQueue: "an event is queued",
  SrvDrop: "an event is dropped at the server",
  SrvLag: "the server's forwarder falls behind",
  SrvWrite: "the writer sends the next item",
  CliEventKept: "an event arrives",
  CliEventFull: "an event arrives while the app's queue is full",
  CliEventSkipped: "an event this build cannot decode arrives",
  CliMissed: "a gap marker arrives",
  Done: "everything settles",
};

export const step = (action: string) => labels[action] ?? action;
