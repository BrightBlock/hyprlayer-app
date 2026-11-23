interface Vars {
  alive: boolean;
  busy: boolean;
  cOpen: string[];
  streaming: boolean;
  vOpen: string[];
  vLive: boolean;
}

export const title = "A composer session's turns, asks and interrupts";

const claude = (v: Vars) =>
  !v.alive ? "claude exited" : v.cOpen.length > 0 ? "claude asks" : v.busy ? "claude works" : "claude idle";

const app = (v: Vars) =>
  !v.vLive ? "the app shows it ended" : v.vOpen.length > 0 ? "the app asks you" : v.streaming ? "the app shows working" : "the app shows idle";

export const state = (v: Vars) => `${claude(v)}, ${app(v)}`;

export const group = (v: Vars) => (!v.alive ? "ended" : v.busy ? "turn" : "idle");

export const colors = { ended: "purple", turn: "blue", idle: "teal" };

const labels: Record<string, string> = {
  ClientSend: "you send a message",
  AgentSend: "an agent sends a message",
  ClientAnswer: "you answer the ask",
  Interrupt: "someone presses stop",
  ClaudeRead: "claude reads its input",
  ClaudeAsk: "claude asks for permission",
  ClaudeTakeQueued: "claude takes in a queued message",
  ClaudeRemoteAnswer: "the ask is answered on claude.ai or by a hook",
  ClaudeToolDone: "the tool allowed elsewhere finishes",
  ClaudeFinish: "claude finishes the turn",
  ClaudeStartQueued: "claude starts the queued message",
  ClaudeExit: "claude exits",
  DriverRead: "the server reads claude's output",
  Relay: "the server stores the event",
  RelayResend: "the server sends a dropped message again",
  ClientRecv: "the app receives the event",
  ClientReload: "the app reloads the transcript",
  ClientResume: "you resume the session",
  Done: "everything settles",
};

export const step = (action: string) => labels[action] ?? action;
