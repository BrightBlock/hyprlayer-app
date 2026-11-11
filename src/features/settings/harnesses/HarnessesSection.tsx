import { useState } from "react";

import type { AppSettings, BinaryStatus } from "@/lib/bindings";
import { harnessLabel } from "@/lib/domain/harnessing";
import { returnFromReauth, useReauth } from "@/lib/reauth";
import { Button, Callout } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { ClaudeHarnessTab } from "./ClaudeHarnessTab";
import { CodexHarnessTab } from "./CodexHarnessTab";
import { OpenCodeHarnessTab } from "./OpenCodeHarnessTab";
import { OllamaHarnessTab } from "./OllamaHarnessTab";

type HarnessTabId = "claude" | "codex" | "opencode" | "ollama";

type TabDef = {
  id: HarnessTabId;
  label: string;
  badge?: string;
};

const TABS: TabDef[] = [
  { id: "claude", label: "Claude Code" },
  { id: "codex", label: "Codex", badge: "NEW" },
  { id: "opencode", label: "OpenCode" },
  { id: "ollama", label: "Ollama" },
];

export function HarnessesSection({
  settings,
  binaries,
}: {
  settings: AppSettings;
  binaries: BinaryStatus[];
}) {
  const reauth = useReauth((s) => s.pending);
  const [active, setActive] = useState<HarnessTabId>(reauth?.harness ?? "claude");
  const opencode = binaries.find((b) => b.kind === "opencode");
  const claude = binaries.find((b) => b.kind === "claude");
  const ollama = binaries.find((b) => b.kind === "ollama");
  const codex = binaries.find((b) => b.kind === "codex");

  return (
    <div className="space-y-4">
      <h1 className="text-h3 text-ink">Harnesses</h1>

      {reauth && (
        <Callout tone="warn" icon={<Icon name="warning" size="sm" />}>
          Sign in to {harnessLabel(reauth.harness)} and your agent resends its last message.
          <Button size="sm" className="ml-2" onClick={returnFromReauth}>
            Back to agent
          </Button>
        </Callout>
      )}

      <div className="flex items-center gap-5 border-b border-edge">
        {TABS.map((tab) => {
          const isActive = active === tab.id;
          return (
            <button
              key={tab.id}
              type="button"
              onClick={() => setActive(tab.id)}
              aria-current={isActive}
              className={`-mb-px flex items-center gap-1.5 border-b-2 px-0.5 pb-2.5 text-body transition-colors ${
                isActive
                  ? "border-accent text-ink"
                  : "border-transparent text-ink-dim hover:text-ink"
              }`}
            >
              {tab.label}
              {tab.badge && (
                <span className="rounded bg-accent/15 px-1 py-0.5 text-tag text-accent">
                  {tab.badge}
                </span>
              )}
            </button>
          );
        })}
      </div>

      {active === "claude" && <ClaudeHarnessTab settings={settings} status={claude} />}
      {active === "opencode" && <OpenCodeHarnessTab settings={settings} status={opencode} />}
      {active === "ollama" && (
        <OllamaHarnessTab
          settings={settings}
          status={ollama}
          opencodeInstalled={opencode?.state.state === "ok"}
        />
      )}
      {active === "codex" && <CodexHarnessTab settings={settings} status={codex} />}
    </div>
  );
}
