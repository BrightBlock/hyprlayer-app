import { useEffect, useRef, useState, type ReactNode } from "react";

import { ArchiveIcon, CheckIcon, PlayIcon, TerminalIcon, WrenchIcon } from "../../components/icons";
import { Badge, Card, CardTitle, ErrorNote, Spinner } from "../../components/ui";
import type { BadgeTone } from "../../components/ui";
import type { ScriptSettings } from "../../lib/bindings";
import { useScripts, useSetScripts } from "../../lib/queries";
import { ScriptEditor } from "./ScriptEditor";

type ScriptKey = keyof ScriptSettings; // "setup" | "run" | "archive"

const EMPTY: ScriptSettings = { setup: "", run: "", archive: "" };
const AUTOSAVE_MS = 500;

type RowMeta = {
  key: ScriptKey;
  name: string;
  trigger: string;
  icon: ReactNode;
  pill: { tone: BadgeTone; label: string; icon?: ReactNode };
  placeholder: string;
};

const ROWS: RowMeta[] = [
  {
    key: "setup",
    name: "Setup",
    trigger: "Runs once when a workstream is created.",
    icon: <WrenchIcon size={13} />,
    pill: { tone: "dim", label: "On create" },
    placeholder: "# install deps & prepare the worktree\nnpm install",
  },
  {
    key: "run",
    name: "Run",
    trigger: "Runs each time you press ▶. The previous process is stopped first.",
    icon: <PlayIcon size={12} />,
    pill: { tone: "accent", label: "On ▶", icon: <PlayIcon size={9} /> },
    placeholder: "# start the dev server for this agent\nnpm run dev -- --port $HL_PORT",
  },
  {
    key: "archive",
    name: "Archive",
    trigger: "Runs before a workstream is archived.",
    icon: <ArchiveIcon size={13} />,
    pill: { tone: "dim", label: "Before archive" },
    placeholder: "# tear down resources before archiving\ndocker compose down",
  },
];

/**
 * Project Settings → "Scripts": edits the three lifecycle command blocks
 * (Setup / Run / Archive). Autosaves to `.hyprlayer/settings.toml` (debounced),
 * with a Saved indicator. The pills here describe each script's trigger — live
 * per-workstream last-run status (Ran 3m ago ✓) shows in the dock, which is
 * workstream-scoped.
 */
export function ScriptsCard({ projectPath }: { projectPath: string }) {
  const scripts = useScripts(projectPath);
  const setScripts = useSetScripts(projectPath);
  const [draft, setDraft] = useState<ScriptSettings | null>(null);

  // Refs so the debounce timer + unmount flush read the latest values without
  // re-subscribing on every keystroke.
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pending = useRef<ScriptSettings | null>(null);
  const mutateRef = useRef(setScripts.mutate);
  mutateRef.current = setScripts.mutate;

  // Seed the local draft once the saved scripts load.
  useEffect(() => {
    if (scripts.data && draft === null) setDraft(scripts.data);
  }, [scripts.data, draft]);

  // Flush any pending save when the dialog closes mid-debounce.
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
      if (pending.current) mutateRef.current(pending.current);
    },
    [],
  );

  const edit = (key: ScriptKey, value: string) => {
    setDraft((d) => {
      const next = { ...(d ?? EMPTY), [key]: value };
      pending.current = next;
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        pending.current = null;
        mutateRef.current(next);
      }, AUTOSAVE_MS);
      return next;
    });
  };

  const dirty =
    !!draft &&
    !!scripts.data &&
    (["setup", "run", "archive"] as ScriptKey[]).some((k) => draft[k] !== scripts.data![k]);
  const saving = setScripts.isPending || dirty;

  return (
    <Card className="shrink-0">
      <div className="mb-3 flex items-center justify-between gap-3">
        <CardTitle className="mb-0">Scripts</CardTitle>
        {draft && (
          <span className="inline-flex items-center gap-1.5 text-xs text-ink-dim">
            {saving ? (
              <>
                <Spinner size={12} /> Saving…
              </>
            ) : (
              <>
                <CheckIcon size={12} className="text-ok" /> Saved
              </>
            )}
          </span>
        )}
      </div>
      <p className="mb-3 text-xs leading-relaxed text-ink-dim">
        Commands that run automatically in <span className="text-ink">every workstream</span>. They
        run in the workstream's shell on your machine. Saved to{" "}
        <code className="font-mono">.hyprlayer/settings.toml</code> (safe to commit).
      </p>

      {scripts.isLoading && <Spinner />}
      {scripts.isError && <ErrorNote error={scripts.error} />}
      {setScripts.isError && <ErrorNote error={setScripts.error} />}

      {draft &&
        ROWS.map((row, i) => (
          <div key={row.key} className={i === 0 ? "" : "mt-4 border-t border-edge pt-4"}>
            <div className="mb-2 flex items-center gap-3">
              <span className="grid h-[26px] w-[26px] shrink-0 place-items-center rounded-lg border border-edge bg-panel-2 text-accent">
                {row.icon}
              </span>
              <div className="min-w-0">
                <span className="text-sm font-semibold text-ink">{row.name}</span>
                <span className="ml-2 text-xs text-ink-dim/70">{row.trigger}</span>
              </div>
              <span className="ml-auto shrink-0">
                <Badge tone={row.pill.tone}>
                  <span className="flex items-center gap-1">
                    {row.pill.icon}
                    {row.pill.label}
                  </span>
                </Badge>
              </span>
            </div>
            <ScriptEditor
              value={draft[row.key] ?? ""}
              onChange={(v) => edit(row.key, v)}
              minRows={3}
              placeholder={row.placeholder}
              aria-label={`${row.name} script`}
            />
            <div className="mt-2 flex items-center gap-1.5 text-xs text-ink-dim/70">
              <TerminalIcon size={11} />
              Runs in the workstream shell
              <span className="ml-auto font-mono">
                working dir: <span className="text-ink-dim">./</span>
              </span>
            </div>
          </div>
        ))}
    </Card>
  );
}
