import { useState } from "react";

import type { AppSettings, BinaryStatus } from "../../../lib/bindings";
import { Badge, Card, CardTitle } from "../../../components/ui";
import { ClaudeHarnessTab } from "./ClaudeHarnessTab";
import { OpenCodeHarnessTab } from "./OpenCodeHarnessTab";
import { OllamaHarnessTab } from "./OllamaHarnessTab";

type HarnessId = "claude" | "codex" | "opencode" | "ollama";

type TabDef = {
  id: HarnessId;
  label: string;
  /** `true` once the harness is real; the others render a "coming soon" panel. */
  ready: boolean;
  badge?: string;
};

const TABS: TabDef[] = [
  { id: "claude", label: "Claude Code", ready: true },
  { id: "codex", label: "Codex", ready: false },
  { id: "opencode", label: "OpenCode", ready: true, badge: "NEW" },
  { id: "ollama", label: "Ollama", ready: true, badge: "NEW" },
];

/** Settings → Harnesses: per-harness configuration behind a horizontal tab
 *  strip. Claude Code and OpenCode are wired up; Codex/Cursor are placeholders. */
export function HarnessesSection({
  settings,
  binaries,
}: {
  settings: AppSettings;
  binaries: BinaryStatus[];
}) {
  const [active, setActive] = useState<HarnessId>("claude");
  const opencode = binaries.find((b) => b.kind === "opencode");
  const claude = binaries.find((b) => b.kind === "claude");
  const ollama = binaries.find((b) => b.kind === "ollama");

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
      {active === "codex" && <ComingSoon name="Codex" />}
    </div>
  );
}

function ComingSoon({ name }: { name: string }) {
  return (
    <Card>
      <div className="flex items-center justify-between">
        <CardTitle className="mb-0">{name}</CardTitle>
        <Badge tone="dim">Coming soon</Badge>
      </div>
      <p className="mt-1 text-xs text-ink-dim">
        {name} support is on the way. Its configuration will appear here once it lands.
      </p>
    </Card>
  );
}
