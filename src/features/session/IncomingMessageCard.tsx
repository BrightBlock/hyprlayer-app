import { Icon, type IconName } from "@/components/Icon";
import { selectAgentTab } from "@/features/search/selectAgentTab";
import { openExternalUrl } from "@/lib/browser";

import type { IncomingLink, IncomingMessage } from "./incomingMessage";
import { focusShell, updateWorkstreamTabs, useWorkstreamTabsStore } from "./workstreamTabs";

const ICON: Record<IncomingMessage["kind"], IconName> = { wake: "wake", agent: "agents", hyprlayer: "server" };

function focusAgent(workstreamId: string, id: string): void {
  const shell = useWorkstreamTabsStore
    .getState()
    .byWs[workstreamId]?.shells.find((s) => s.sessionId === id || s.ptyAgent?.terminalId === id);
  if (!shell) {
    void selectAgentTab(workstreamId, id);
    return;
  }
  updateWorkstreamTabs((all) => {
    const ws = all[workstreamId];
    return ws ? { ...all, [workstreamId]: focusShell(ws, shell.id) } : all;
  });
}

function LinkButton({ link, workstreamId }: { link: IncomingLink; workstreamId: string | null }) {
  if (link.kind === "agent" && !workstreamId) return null;
  const onClick = () =>
    link.kind === "pr" ? void openExternalUrl(link.url) : focusAgent(workstreamId as string, link.id);
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

export function IncomingMessageCard({
  message,
  workstreamId,
}: {
  message: IncomingMessage;
  workstreamId: string | null;
}) {
  const { kind, title, body, ids, links } = message;
  return (
    <div className="flex flex-col gap-1">
      <div className="flex min-w-0 items-center gap-1.5 text-ui-sm">
        <span className="flex min-w-0 items-center gap-1.5" title={ids.join("\n") || undefined}>
          <Icon name={ICON[kind]} size="xs" className="shrink-0 text-ink-dim" />
          <span className="truncate font-medium text-ink">{title}</span>
        </span>
        {links.map((link) => (
          <LinkButton key={link.kind === "pr" ? link.url : link.id} link={link} workstreamId={workstreamId} />
        ))}
      </div>
      {body && <p className="selectable whitespace-pre-wrap break-words text-body text-ink">{body}</p>}
    </div>
  );
}
