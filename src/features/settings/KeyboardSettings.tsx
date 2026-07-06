import { useEffect, useMemo, useState } from "react";

import { Button, Card, CardTitle, ErrorNote } from "@/components/ui";
import { useSetAppSettings } from "@/lib/queries";
import { IS_MACOS } from "@/lib/platform";
import type { AppSettings } from "@/lib/bindings";
import {
  eventToChord,
  formatChord,
  normalizeChord,
  type Chord,
} from "@/lib/keybindings/chord";
import {
  COMMANDS_BY_CATEGORY,
  commandById,
  type CommandDef,
  type CommandWhen,
} from "@/lib/keybindings/commands";
import {
  conflictsFor,
  effectiveChords,
  hasOverride,
  type Overrides,
} from "@/lib/keybindings/resolve";
import { isReserved } from "@/lib/keybindings/reserved";

/** Stable identity for "no overrides" so the effective-chords memo doesn't churn every render. */
const EMPTY_OVERRIDES: Overrides = {};

/** Short context hint shown beside each command. */
const WHEN_HINT: Record<CommandWhen, string> = {
  always: "Everywhere",
  project: "In a project",
  workstream: "In a workstream",
  session: "In a session",
};

/** A small monospace chord pill, or a muted "Unbound" when null/empty. */
function ChordPill({ chord }: { chord: Chord | null }) {
  if (!chord) {
    return <span className="text-xs italic text-ink-dim">Unbound</span>;
  }
  return (
    // Sized to match the Edit/Reset/Unbind buttons so the row reads as one set of
    // equal controls. No `font-mono`: the system font renders ⌘/⇧ at native size.
    <kbd
      className="inline-flex items-center rounded-md border border-edge bg-panel-2 px-2.5 py-1.5 text-sm font-medium text-ink"
      // macOS concatenates modifier glyphs (⌘⇧B) with no separators — track them
      // apart so each stays legible; the +-joined Windows/Linux form needs none.
      style={IS_MACOS ? { letterSpacing: "0.1em", paddingRight: "calc(0.625rem - 0.1em)" } : undefined}
    >
      {formatChord(chord, IS_MACOS)}
    </kbd>
  );
}

function CommandRow({
  cmd,
  effective,
  overrides,
  recording,
  onStartRecord,
  onCommit,
  onReset,
  onUnbind,
  onCancel,
}: {
  cmd: CommandDef;
  effective: Chord | null;
  overrides: Overrides;
  recording: boolean;
  onStartRecord: () => void;
  onCommit: (chord: Chord) => void;
  onReset: () => void;
  onUnbind: () => void;
  onCancel: () => void;
}) {
  const [candidate, setCandidate] = useState<Chord | null>(null);

  // Capture the chord on the capture phase so it beats the global dispatcher
  // (bubble phase) and never fires a shortcut mid-record. Esc cancels, Enter confirms.
  useEffect(() => {
    if (!recording) {
      setCandidate(null);
      return;
    }
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        onCancel();
        return;
      }
      if (
        e.key === "Enter" &&
        !e.metaKey &&
        !e.ctrlKey &&
        !e.altKey &&
        !e.shiftKey &&
        candidate
      ) {
        onCommit(candidate);
        return;
      }
      const chord = eventToChord(e, IS_MACOS);
      if (chord) setCandidate(chord);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording, candidate, onCancel, onCommit]);

  const conflicts = candidate ? conflictsFor(candidate, cmd.id, overrides, IS_MACOS) : [];
  const reserved = candidate ? isReserved(candidate, IS_MACOS) : false;

  return (
    <div className="flex items-start justify-between gap-4 py-3">
      <div className="min-w-0">
        <p className="text-sm text-ink">{cmd.label}</p>
        <p className="mt-0.5 text-xs text-ink-dim">{WHEN_HINT[cmd.when]}</p>
      </div>

      <div className="flex shrink-0 flex-col items-end gap-1.5">
        {recording ? (
          <>
            <div className="flex items-center gap-2">
              {candidate ? (
                <ChordPill chord={candidate} />
              ) : (
                <span className="text-xs text-ink-dim">Press a shortcut…</span>
              )}
              <Button
                variant="primary"
                disabled={!candidate}
                onClick={() => candidate && onCommit(candidate)}
              >
                Save
              </Button>
              <Button onClick={onCancel}>Cancel</Button>
            </div>
            {conflicts.length > 0 && (
              <p className="text-right text-xs text-warn">
                Already used by{" "}
                {conflicts.map((id) => commandById(id)?.label ?? id).join(", ")}
              </p>
            )}
            {reserved && (
              <p className="text-right text-xs text-warn">
                This is a system shortcut — the OS may intercept it.
              </p>
            )}
          </>
        ) : (
          <div className="flex items-center gap-2">
            <ChordPill chord={effective} />
            <Button onClick={onStartRecord}>Edit</Button>
            <Button onClick={onReset} disabled={!hasOverride(overrides, cmd.id)}>
              Reset
            </Button>
            <Button onClick={onUnbind} disabled={effective === null}>
              Unbind
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

export function KeyboardSettings({ settings }: { settings: AppSettings }) {
  const save = useSetAppSettings();
  const [recordingId, setRecordingId] = useState<string | null>(null);

  const overrides: Overrides = settings.keybindings?.overrides ?? EMPTY_OVERRIDES;
  const effective = useMemo(() => effectiveChords(overrides, IS_MACOS), [overrides]);

  function persist(next: Overrides) {
    save.mutate({ ...settings, keybindings: { overrides: next } });
  }

  function commit(id: string, chord: Chord) {
    persist({ ...overrides, [id]: normalizeChord(chord) });
    setRecordingId(null);
  }

  function reset(id: string) {
    const next = { ...overrides };
    delete next[id];
    persist(next);
  }

  function unbind(id: string) {
    persist({ ...overrides, [id]: null });
  }

  return (
    <div className="space-y-4">
      <div>
        <p className="text-xs text-ink-dim">
          Customize keyboard shortcuts. Shortcuts only fire in the context shown
          beside each command, and never while you're typing in the composer or an
          editor. Reset restores the default; Unbind clears the shortcut.
        </p>
      </div>

      {save.isError && <ErrorNote error={save.error} />}

      {COMMANDS_BY_CATEGORY.map(({ category, commands }) => (
        <Card key={category}>
          <CardTitle>{category}</CardTitle>
          <div className="divide-y divide-edge/50">
            {commands.map((cmd) => (
              <CommandRow
                key={cmd.id}
                cmd={cmd}
                effective={effective.get(cmd.id) ?? null}
                overrides={overrides}
                recording={recordingId === cmd.id}
                onStartRecord={() => setRecordingId(cmd.id)}
                onCommit={(chord) => commit(cmd.id, chord)}
                onReset={() => reset(cmd.id)}
                onUnbind={() => unbind(cmd.id)}
                onCancel={() => setRecordingId(null)}
              />
            ))}
          </div>
        </Card>
      ))}
    </div>
  );
}
