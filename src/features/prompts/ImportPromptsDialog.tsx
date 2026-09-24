import { useMemo, useState } from "react";

import { Badge, Button, Dialog } from "@/components/ui";
import { DocIcon, ImportIcon, WarningTriangleIcon } from "@/components/icons";
import { PromptRowIcon, ShareCheck } from "./promptShareUi";
import { promptFirstLine } from "./prompts";
import type { PromptConflictAction } from "@/lib/bindings";
import { willImportCount, type ImportDecision, type ImportRow } from "./promptsIo";

const RESOLUTIONS: { value: PromptConflictAction; label: string }[] = [
  { value: "keepBoth", label: "Keep both" },
  { value: "replace", label: "Replace yours" },
  { value: "skip", label: "Skip" },
];

function rowNotes(c: ImportRow, resolution: PromptConflictAction, chordLabel: (c: string) => string): string[] {
  const notes: string[] = [];
  const chordShown = !c.conflict || resolution !== "replace";
  if (c.chordTaken && c.chord && chordShown) {
    notes.push(
      `${chordLabel(c.chord)} is already assigned in your library. This copy imports without a shortcut.`,
    );
  }
  if (c.unknownProjects.length > 0) {
    notes.push(
      `Includes a project override for “${c.unknownProjects.join("”, “")}”. No matching project here, so it will be dropped.`,
    );
  }
  return notes;
}

export function ImportPromptsDialog({
  fileName,
  source,
  rows,
  chordLabel,
  onConfirm,
  onChooseAnother,
  onClose,
}: {
  fileName: string;
  source: string | null;
  rows: ImportRow[];
  chordLabel: (chord: string) => string;
  onConfirm: (decisions: Record<string, ImportDecision>) => void;
  onChooseAnother: () => void;
  onClose: () => void;
}) {
  const [decisions, setDecisions] = useState<Record<string, ImportDecision>>(() =>
    Object.fromEntries(rows.map((c) => [c.key, { include: true, resolution: "keepBoth" }])),
  );

  const setDecision = (key: string, patch: Partial<ImportDecision>) =>
    setDecisions((d) => ({ ...d, [key]: { ...d[key], ...patch } }));

  const willImport = useMemo(() => willImportCount(rows, decisions), [rows, decisions]);

  return (
    <Dialog
      open
      onClose={onClose}
      title={
        <span className="flex items-center gap-2.5">
          <ImportIcon size={15} className="shrink-0 text-accent" />
          Import prompts
        </span>
      }
      size="xl"
      align="top"
      seams
      padded={false}
      closable
      footer={
        <div className="flex items-center gap-2.5">
          <span className="text-[11px] text-ink-dim">
            {willImport} of {rows.length} will be imported
          </span>
          <div className="ml-auto flex items-center gap-2">
            <Button onClick={onClose}>Cancel</Button>
            <Button variant="primary" disabled={willImport === 0} onClick={() => onConfirm(decisions)}>
              Import {willImport} prompt{willImport === 1 ? "" : "s"}
            </Button>
          </div>
        </div>
      }
    >
      <div className="flex items-center gap-2 border-b border-edge bg-panel-2/40 px-4 py-2.5">
        <DocIcon size={14} className="shrink-0 text-ink-dim" />
        <span className="truncate font-mono text-[11px] text-ink">{fileName}</span>
        {source && <span className="shrink-0 text-[11px] text-ink-dim">from {source}</span>}
        <button
          type="button"
          onClick={onChooseAnother}
          className="ml-auto shrink-0 text-[11px] text-ink-dim underline underline-offset-2 hover:text-ink"
        >
          Choose another file
        </button>
      </div>

      <div className="min-h-0 flex-1 space-y-2 overflow-y-auto px-4 py-3">
        {rows.map((c) => {
          const d = decisions[c.key] ?? { include: true, resolution: "keepBoth" as PromptConflictAction };
          const skipped = Boolean(c.conflict) && d.resolution === "skip";
          const notes = d.include && !skipped ? rowNotes(c, d.resolution, chordLabel) : [];
          const showConflict = Boolean(c.conflict) && d.include;
          return (
            <div
              key={c.key}
              className={`space-y-2 rounded-lg border bg-panel-2 px-3 py-2.5 transition-opacity ${
                c.conflict && d.include ? "border-warn/40" : "border-edge"
              } ${d.include && !skipped ? "" : "opacity-50"}`}
            >
              <div className="flex items-center gap-2.5">
                <ShareCheck
                  checked={d.include}
                  onToggle={() => setDecision(c.key, { include: !d.include })}
                  label={`Import ${c.name}`}
                />
                <PromptRowIcon builtin={false} id="" className="shrink-0 text-ink-dim" />
                <span className="truncate text-sm font-medium text-ink">{c.name}</span>
                {c.chord && (
                  <span className="shrink-0 rounded border border-edge bg-surface px-1 font-mono text-[10px] text-ink-dim">
                    {chordLabel(c.chord)}
                  </span>
                )}
                <span className="ml-auto shrink-0">
                  {c.conflict ? <Badge tone="warn">Name conflict</Badge> : <Badge tone="ok">New</Badge>}
                </span>
              </div>

              <p className="truncate pl-[26px] text-[11px] text-ink-dim">
                {promptFirstLine(c.body) || "No prompt text"}
              </p>

              {showConflict && (
                <div className="flex flex-wrap items-center gap-2 pl-[26px]">
                  <span className="text-[11px] text-warn-ink">“{c.name}” already exists:</span>
                  <div className="inline-flex gap-1">
                    {RESOLUTIONS.map((r) => {
                      const active = d.resolution === r.value;
                      return (
                        <button
                          key={r.value}
                          type="button"
                          onClick={() => setDecision(c.key, { resolution: r.value })}
                          className={`rounded px-2 py-0.5 text-[11px] font-medium transition-colors ${
                            active
                              ? "border border-accent bg-accent text-accent-ink"
                              : "border border-edge bg-panel text-ink-dim hover:text-ink"
                          }`}
                        >
                          {r.label}
                        </button>
                      );
                    })}
                  </div>
                </div>
              )}

              {notes.map((note, i) => (
                <div key={i} className="flex items-start gap-2 pl-[26px] text-[11px] leading-relaxed text-ink-dim">
                  <WarningTriangleIcon size={12} className="mt-0.5 shrink-0 text-warn-ink" />
                  <span>{note}</span>
                </div>
              ))}
            </div>
          );
        })}
      </div>
    </Dialog>
  );
}
