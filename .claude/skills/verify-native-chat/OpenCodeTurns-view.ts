interface Vars {
  alive: boolean;
  running: boolean;
  cOpen: string[];
  streaming: boolean;
  vOpen: string[];
}

export const title = "A composer session's turns, steers, asks and stops on OpenCode 2";

const server = (v: Vars) =>
  !v.alive ? "OpenCode is down" : v.cOpen.length > 0 ? "OpenCode asks" : v.running ? "OpenCode runs" : "OpenCode idle";

const app = (v: Vars) =>
  v.vOpen.length > 0 ? "the app asks you" : v.streaming ? "the app shows working" : "the app shows idle";

export const state = (v: Vars) => `${server(v)}, ${app(v)}`;

export const group = (v: Vars) => (!v.alive ? "down" : v.running ? "turn" : "idle");

export const colors = { down: "purple", turn: "blue", idle: "teal" };

const labels: Record<string, string> = {
  ClientSend: "you send a message, or steer one into the running turn",
  AgentSend: "an agent sends a message",
  ClientAnswer: "you answer the ask",
  Interrupt: "someone presses stop",
  OcPost: "OpenCode handles the server's next request",
  OcWake: "a subagent's result wakes the OpenCode session",
  OcDeliver: "OpenCode's run reads the next inbox message and answers it",
  OcAsk: "OpenCode asks for permission",
  OcElsewhereReply: "the ask is answered in another OpenCode client",
  OcFinish: "OpenCode's run ends",
  OcDown: "the OpenCode server dies",
  ServerExited: "the server reports the OpenCode server's death",
  SseDrop: "an event never reaches the server",
  DriverRead: "the server reads OpenCode's event",
  Resync: "the server reads back what OpenCode holds after the stream reconnects",
  Relay: "the server stores the event",
  RelayExited: "the server drops the session's driver",
  RelayResend: "the server sends a handed-back message again",
  ClientReload: "the app reloads the transcript",
  Done: "everything settles",
};

export const step = (action: string) => labels[action] ?? action;
