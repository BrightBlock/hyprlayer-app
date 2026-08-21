import { useMemo, useState } from "react";

import { Badge, Button, Dialog } from "@/components/ui";
import { ExportIcon } from "@/components/icons";
import { PromptRowIcon, ShareCheck } from "./promptShareUi";
import type { ExportEntry, ExportOptions } from "./promptsIo";
import { PROMPTS_FILE_EXT } from "./promptsIo";

/** The default multi-prompt export bundle name (matches the mock). */
export const DEFAULT_EXPORT_NAME = `hyprlayer-prompts.${PROMPTS_FILE_EXT}`;

/**
 * The "Export prompts" dialog, opened from the prompt library's sidebar header.
 * Pick which prompts (built-in and custom) to bundle, whether to carry keyboard
 * shortcuts and project overrides, then hand the selection back to the page,
 * which drives the native save dialog and writes the file.
 *
 * Mounted only while open, so its selection state resets each time it opens.
 */
export function ExportPromptsDialog({
  entries,
  chordLabel,
  onExport,
  onClose,
}: {
  entries: ExportEntry[];
  chordLabel: (chord: string) => string;
  onExport: (selected: ExportEntry[], opts: ExportOptions) => void;
  onClose: () => void;
}) {
  const [checks, setChecks] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(entries.map((e) => [e.id, true])),
  );
  const [includeChords, setIncludeChords] = useState(true);
  const [includeOverrides, setIncludeOverrides] = useState(false);

  const selected = useMemo(() => entries.filter((e) => checks[e.id]), [entries, checks]);
  const count = selected.length;

  return (
    <Dialog
      open
      onClose={onClose}
      title={
        <span className="flex items-center gap-2.5">
          <ExportIcon size={15} className="shrink-0 text-accent" />
          Export prompts
        </span>
      }
      size="lg"
      align="top"
      seams
      padded={false}
      closable
      footer={
        <div className="space-y-3">
          <div className="space-y-2.5">
            <label className="flex cursor-pointer items-center gap-2.5">
              <ShareCheck
                checked={includeChords}
                onToggle={() => setIncludeChords((v) => !v)}
                label="Include keyboard shortcuts"
              />
              <span className="text-xs text-ink">Include keyboard shortcuts</span>
            </label>
            <div className="space-y-1">
              <label className="flex cursor-pointer items-center gap-2.5">
                <ShareCheck
                  checked={includeOverrides}
                  onToggle={() => setIncludeOverrides((v) => !v)}
                  label="Include project overrides"
                />
                <span className="text-xs text-ink">Include project overrides</span>
              </label>
              <p className="pl-[26px] text-[11px] leading-relaxed text-ink-dim">
                Overrides only apply if the importer has a project with the same name.
              </p>
            </div>
          </div>
          <div className="flex items-center gap-2.5 border-t border-edge pt-3">
            <span className="truncate font-mono text-[11px] text-ink-dim">
              {DEFAULT_EXPORT_NAME}
            </span>
            <div className="ml-auto flex items-center gap-2">
              <Button onClick={onClose}>Cancel</Button>
              <Button
                variant="primary"
                disabled={count === 0}
                onClick={() => onExport(selected, { includeChords, includeOverrides })}
              >
                Export {count} prompt{count === 1 ? "" : "s"}
              </Button>
            </div>
          </div>
        </div>
      }
    >
      <div className="border-b border-edge px-4 pb-3 pt-0.5">
        <p className="text-xs leading-relaxed text-ink-dim">
          Writes a .{PROMPTS_FILE_EXT} file anyone on Hyprlayer desktop can import.
        </p>
      </div>

      {/* Prompt checklist */}
      <div className="min-h-0 flex-1 space-y-1 overflow-y-auto px-4 py-3">
        {entries.length === 0 && (
          <div className="rounded-md border border-dashed border-edge px-3 py-5 text-center text-xs leading-relaxed text-ink-dim">
            Nothing to export yet. Add a custom prompt or customize a built-in first.
          </div>
        )}
        {entries.map((e) => {
          const checked = Boolean(checks[e.id]);
          const overrideCount = Object.keys(e.overrides).length;
          return (
            <div
              key={e.id}
              className={`flex items-center gap-2.5 rounded-md border border-edge bg-panel-2 px-2.5 py-2 transition-opacity ${
                checked ? "" : "opacity-50"
              }`}
            >
              <ShareCheck
                checked={checked}
                onToggle={() => setChecks((c) => ({ ...c, [e.id]: !checked }))}
                label={`Include ${e.name}`}
              />
              <PromptRowIcon builtin={e.builtin} id={e.id} className="shrink-0 text-ink-dim" />
              <div className="flex min-w-0 flex-1 items-center gap-2">
                <span className="truncate text-sm text-ink">{e.name}</span>
                {e.builtin && <Badge tone="dim">Built-in</Badge>}
                {overrideCount > 0 && (
                  <span className="shrink-0 rounded bg-brand/15 px-1 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-brand">
                    {overrideCount} override
                  </span>
                )}
              </div>
              {e.chord && (
                <span className="ml-2 shrink-0 font-mono text-[11px] text-ink-dim">
                  {chordLabel(e.chord)}
                </span>
              )}
            </div>
          );
        })}
      </div>
    </Dialog>
  );
}
