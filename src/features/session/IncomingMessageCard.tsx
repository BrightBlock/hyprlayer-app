import { Icon, type IconName } from "@/components/Icon";
import { reattachPtyAgent } from "@/features/multiplexer/ptyAgentBridge";
import { selectAgentTab } from "@/features/search/selectAgentTab";
import { goToWorkstream } from "@/lib/appNav";
import { openExternalUrl } from "@/lib/browser";
import { commands } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { workstreamSessionsQuery } from "@/lib/queries";
import { queryClient } from "@/lib/queryClient";
import { useToast } from "@/lib/toast";

import type { IncomingLink, IncomingMessage } from "./incomingMessage";
import { focusShell, updateWorkstreamTabs, useWorkstreamTabsStore } from "./workstreamTabs";

const ICON: Record<IncomingMessage["kind"], IconName> = { wake: "wake", agent: "agents", hyprlayer: "server" };

type Sender = { workstreamId: string; open: () => void };

async function findSender(id: string, workstreamIds: string[]): Promise<Sender | null> {
  for (const [workstreamId, tabs] of Object.entries(useWorkstreamTabsStore.getState().byWs)) {
    const shell = tabs.shells.find((s) => s.sessionId === id || s.ptyAgent?.terminalId === id);
    if (shell) {
      const open = () =>
        updateWorkstreamTabs((all) => {
          const ws = all[workstreamId];
          return ws ? { ...all, [workstreamId]: focusShell(ws, shell.id) } : all;
        });
      return { workstreamId, open };
    }
  }
  const terminal = (await unwrap(commands.listPtyAgents(null))).find((a) => a.terminalId === id);
  if (terminal) return { workstreamId: terminal.workstreamId, open: () => reattachPtyAgent(terminal) };
  const lists = await Promise.all(
    workstreamIds.map((wsId) => queryClient.fetchQuery(workstreamSessionsQuery(wsId)).catch(() => [])),
  );
  const workstreamId = workstreamIds.find((_, i) => lists[i].some((s) => s.id === id));
  return workstreamId ? { workstreamId, open: () => void selectAgentTab(workstreamId, id) } : null;
}

async function openSender(id: string): Promise<void> {
  const workstreams = await unwrap(commands.listAllWorkstreams());
  const sender = await findSender(
    id,
    workstreams.map((w) => w.item.id),
  );
  const view = sender && workstreams.find((w) => w.item.id === sender.workstreamId);
  if (!sender || !view) {
    useToast.getState().show("Agent not found", "It has ended, or its workstream is archived.", { tone: "warn" });
    return;
  }
  goToWorkstream({ projectPath: view.item.projectPath, workstreamId: view.item.id });
  sender.open();
}

function LinkButton({ link }: { link: IncomingLink }) {
  const onClick = () => {
    if (link.kind === "pr") void openExternalUrl(link.url);
    else openSender(link.id).catch((e) => log.warn("incoming-message", "the sender did not open", e));
  };
  return (
    <button
      type="button"
      onClick={onClick}
      title={link.kind === "pr" ? link.url : link.id}
      className="inline-flex shrink-0 items-center gap-0.5 rounded-control text-ui-sm text-info hover:underline"
    >
      {link.label}
      <Icon name={link.kind === "pr" ? "external" : "arrowRight"} size="xs" />
    </button>
  );
}

export function IncomingMessageCard({ message }: { message: IncomingMessage }) {
  const { kind, title, body, ids, links } = message;
  return (
    <div className="flex flex-col gap-1">
      <div className="flex min-w-0 items-center gap-1.5 text-ui-sm">
        <span className="flex min-w-0 items-center gap-1.5" title={ids.join("\n") || undefined}>
          <Icon name={ICON[kind]} size="xs" className="shrink-0 text-ink-dim" />
          <span className="truncate font-medium text-ink">{title}</span>
        </span>
        {links.map((link) => (
          <LinkButton key={link.kind === "pr" ? link.url : link.id} link={link} />
        ))}
      </div>
      {body && <p className="selectable whitespace-pre-wrap break-words text-body text-ink">{body}</p>}
    </div>
  );
}
