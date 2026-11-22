interface Vars {
  entry: string;
  task: string;
  rx: string;
  inbox: string;
  credit: number;
}

export const title = "One stream's credit and cancel";

export const state = (v: Vars) => {
  const server =
    v.entry === "open"
      ? v.credit >= 1
        ? "server streaming"
        : "server waiting for credit"
      : v.task === "aborting"
        ? "server finishing its last frame"
        : v.entry === "none" && v.task === "unborn"
          ? "server preparing"
          : v.entry.startsWith("reserved")
            ? "server about to start"
            : "server stopped";
  const client =
    v.rx === "alive"
      ? "you are reading"
      : v.rx === "dropped"
        ? v.inbox === "open"
          ? "you stopped; the inbox is still held"
          : "you stopped"
        : "waiting for the answer";
  return `${server}, ${client}`;
};

export const group = (v: Vars) => (v.rx === "alive" ? "reading" : v.rx === "dropped" ? "stopped" : "opening");

export const colors = { opening: "blue", reading: "teal", stopped: "purple" };

const labels: Record<string, string> = {
  SrvRespond: "the server answers with the stream id",
  SrvSpawn: "the server starts forwarding",
  SrvReady: "the server has credit for a frame",
  SrvPush: "the server sends a frame",
  SrvSourceEnds: "the source ends",
  SrvReadCredit: "the server reads your credit",
  SrvReadCancel: "the server reads your cancel",
  CliReadResp: "the answer arrives",
  CliReadData: "a frame arrives",
  CliRecv: "you read a frame",
  CliDrop: "you stop reading",
  Done: "everything settles",
};

export const step = (action: string) => labels[action] ?? action;
