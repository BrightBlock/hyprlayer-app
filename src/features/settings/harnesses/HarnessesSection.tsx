import { useState } from "react";

import type { AppSettings, BinaryStatus } from "@/lib/bindings";
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
  const [active, setActive] = useState<HarnessTabId>("claude");
  const opencode = binaries.find((b) => b.kind === "opencode");
  const claude = binaries.find((b) => b.kind === "claude");
  const ollama = binaries.find((b) => b.kind === "ollama");
  const codex = binaries.find((b) => b.kind === "codex");

  return (
    <div className="space-y-4">
      <h1 className="text-2xl font-semibold tracking-tight text-ink">Harnesses</h1>

      <div className="flex items-center gap-5 border-b border-edge">
        {TABS.map((tab) => {
          const isActive = active === tab.id;
          return (
            <button
              key={tab.id}
              type="button"
              onClick={() => setActive(tab.id)}
              aria-current={isActive}
              className={`-mb-px flex items-center gap-1.5 border-b-2 px-0.5 pb-2.5 text-sm transition-colors ${
                isActive
                  ? "border-accent text-ink"
                  : "border-transparent text-ink-dim hover:text-ink"
              }`}
            >
              {tab.label}
              {tab.badge && (
                <span className="rounded bg-accent/15 px-1 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-accent">
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
