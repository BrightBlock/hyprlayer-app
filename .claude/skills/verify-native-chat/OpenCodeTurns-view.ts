interface Vars {
  alive: boolean;
  busy: boolean;
  cOpen: string[];
  streaming: boolean;
  vOpen: string[];
}

export const title = "A composer session's turns, asks and interrupts on OpenCode";

const server = (v: Vars) =>
  !v.alive ? "OpenCode is down" : v.cOpen.length > 0 ? "OpenCode asks" : v.busy ? "OpenCode works" : "OpenCode idle";

const app = (v: Vars) =>
  v.vOpen.length > 0 ? "the app asks you" : v.streaming ? "the app shows working" : "the app shows idle";

export const state = (v: Vars) => `${server(v)}, ${app(v)}`;

export const group = (v: Vars) => (!v.alive ? "down" : v.busy ? "turn" : "idle");

export const colors = { down: "purple", turn: "blue", idle: "teal" };

const labels: Record<string, string> = {
  ClientSend: "you send a message",
  AgentSend: "an agent sends a message",
  ClientAnswer: "you answer the ask",
  Interrupt: "someone presses stop",
  OcRead: "OpenCode handles a request",
  OcAsk: "OpenCode asks for permission",
  OcElsewhereReply: "the ask is answered in another OpenCode client",
  OcFinish: "OpenCode finishes the reply",
  OcDown: "the OpenCode server dies",
  ServerExited: "the server reports the OpenCode server's death",
  SseDrop: "an event never reaches the server",
  DriverRead: "the server reads OpenCode's event",
  Expire: "an unclaimed idle ends the turn",
  Settle: "the held turn end goes out",
  Relay: "the server stores the event",
  RelayExited: "the server drops the session's driver",
  ClientReload: "the app reloads the transcript",
  Done: "everything settles",
};

export const step = (action: string) => labels[action] ?? action;
