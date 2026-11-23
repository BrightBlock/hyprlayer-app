interface Vars {
  alive: boolean;
  busy: boolean;
  cOpen: string[];
  streaming: boolean;
  vSeq: Record<string, string>;
  vLive: boolean;
}

export const title = "A composer session's turns, asks and interrupts on codex";

const codex = (v: Vars) =>
  !v.alive ? "codex exited" : v.cOpen.length > 0 ? "codex asks" : v.busy ? "codex works" : "codex idle";

const app = (v: Vars) =>
  !v.vLive
    ? "the app shows it ended"
    : Object.keys(v.vSeq ?? {}).length > 0
      ? "the app asks you"
      : v.streaming
        ? "the app shows working"
        : "the app shows idle";

export const state = (v: Vars) => `${codex(v)}, ${app(v)}`;

export const group = (v: Vars) => (!v.alive ? "ended" : v.busy ? "turn" : "idle");

export const colors = { ended: "purple", turn: "blue", idle: "teal" };

const labels: Record<string, string> = {
  ClientSend: "you send a message",
  AgentSend: "an agent sends a message",
  ClientAnswer: "you answer the ask",
  Interrupt: "someone presses stop",
  CodexRead: "codex reads a request",
  CodexBegin: "codex starts the turn",
  CodexAsk: "codex asks for approval",
  CodexOtherTool: "another command finishes",
  CodexCommitSteer: "codex takes in a steered message",
  CodexFinish: "codex finishes the turn",
  CodexExit: "codex exits",
  DriverRead: "the server reads codex's output",
  Relay: "the server stores the event",
  ClientReload: "the app reloads the transcript",
  ClientResume: "you resume the session",
  Done: "everything settles",
};

export const step = (action: string) => labels[action] ?? action;
