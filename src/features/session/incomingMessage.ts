export type IncomingLink =
  | { kind: "pr"; label: string; url: string }
  | { kind: "agent"; label: string; id: string };

export type IncomingMessage = {
  kind: "wake" | "agent" | "hyprlayer";
  title: string;
  body: string;
  ids: string[];
  links: IncomingLink[];
};

const WAKE = /^\[from hyprlayer · wake ([^\]\s]+)\]$/;
const HYPRLAYER = /^\[from hyprlayer\]$/;
const AGENT = /^\[from agent "(.*)" · ([^\]\s]+)\]$/;
const AGENT_REF = /\b(session|terminal) ([0-9a-f][0-9a-f-]{7,})(?: "((?:[^"\\]|\\.)*)")?/g;
const PR = /^(.*?) — (https?:\/\/\S+)$/;

function split(text: string): { header: string; rest: string; trailing: boolean } {
  const first = text.indexOf("\n");
  const head = first === -1 ? text : text.slice(0, first);
  if (head.startsWith("[from ")) return { header: head, rest: first === -1 ? "" : text.slice(first + 1), trailing: false };
  const last = text.lastIndexOf("\n");
  return { header: text.slice(last + 1), rest: text.slice(0, Math.max(last, 0)), trailing: true };
}

function wakeReport(report: string, wakeId: string): IncomingMessage {
  const [line = "", ...more] = report.split("\n");
  const ids = [`wake ${wakeId}`];
  const links: IncomingLink[] = [];
  let summary = line.replace(AGENT_REF, (_, kind: string, id: string, title?: string) => {
    ids.push(`${kind} ${id}`);
    links.push({ kind: "agent", label: kind === "session" ? "Open session" : "Open terminal", id });
    return title ?? kind;
  });
  const pr = PR.exec(summary);
  if (pr) {
    summary = pr[1];
    links.push({ kind: "pr", label: summary.match(/^PR #\d+/)?.[0] ?? "Open PR", url: pr[2] });
  }
  return { kind: "wake", title: `Wake · ${summary}`, body: more.join("\n").trim(), ids, links };
}

export function parseIncomingMessage(text: string): IncomingMessage | null {
  if (!text.includes("[from ")) return null;
  const { header, rest, trailing } = split(text.trim());
  const body = rest.trim();
  if (!trailing) {
    const wake = WAKE.exec(header);
    if (wake) return wakeReport(body, wake[1]);
    if (HYPRLAYER.test(header)) return { kind: "hyprlayer", title: "Hyprlayer", body, ids: [], links: [] };
  }
  const agent = AGENT.exec(header);
  if (!agent) return null;
  const [, label, id] = agent;
  return { kind: "agent", title: label, body, ids: [id], links: [{ kind: "agent", label: "Open agent", id }] };
}
