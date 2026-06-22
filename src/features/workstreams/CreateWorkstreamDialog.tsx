import { useEffect, useMemo, useState } from "react";

import type { SeedReport, WorkstreamView } from "../../lib/bindings";
import { Button, Dialog, ErrorNote, Field, Select, TextInput } from "../../components/ui";
import { useCreateWorkstream, useWorkstreamBases } from "../../lib/queries";

type Props = {
  projectPath: string;
  open: boolean;
  onClose: () => void;
  /** Called with the freshly created item so the caller can select its tab. */
  onCreated: (view: WorkstreamView) => void;
};

/**
 * Title prompt for a new workstream. Creation provisions a branch + worktree
 * on disk (~1-3s), so the dialog stays open with a busy state until done.
 * The base branch defaults to the repo's detected default; pick another to
 * branch from a release line or feature branch instead.
 */
/** Brief non-blocking note shown when the seed step had missing/errored files. */
function SeedNote({ seed }: { seed: SeedReport }) {
  const problems = [...seed.missing, ...seed.errors.map((e) => e.entry)];
  if (problems.length === 0) return null;
  return (
    <p className="rounded-md border border-warn/30 bg-warn/10 px-3 py-2 text-xs text-warn">
      Copied {seed.copied.length} file{seed.copied.length !== 1 ? "s" : ""};
      couldn&apos;t find: {problems.join(", ")}
    </p>
  );
}

export function CreateWorkstreamDialog({ projectPath, open, onClose, onCreated }: Props) {
  const [title, setTitle] = useState("");
  const [base, setBase] = useState("");
  const [seedNote, setSeedNote] = useState<SeedReport | null>(null);
  const create = useCreateWorkstream(projectPath);
  const bases = useWorkstreamBases(projectPath, open);

  // Pre-select the detected default once the branch list loads (unless the user
  // already chose one), falling back to the first branch when detection turns
  // up nothing — so the controlled <select> always matches a real option.
  // Keyed on the dialog being open so reopening re-defaults.
  const defaultBase = bases.data?.defaultBase ?? "";
  // Memoized so its identity is stable across renders — otherwise the `?? []`
  // literal would re-fire the default-selection effect every render.
  const branches = useMemo(() => bases.data?.branches ?? [], [bases.data?.branches]);
  useEffect(() => {
    if (!open || base) return;
    const initial = defaultBase || branches[0];
    if (initial) setBase(initial);
  }, [open, base, defaultBase, branches]);

  const close = () => {
    if (create.isPending) return;
    setTitle("");
    setBase("");
    setSeedNote(null);
    create.reset();
    onClose();
  };

  const submit = () => {
    const trimmed = title.trim();
    if (!trimmed || create.isPending) return;
    create.mutate(
      { title: trimmed, base: base || null },
      {
        onSuccess: (view) => {
          setTitle("");
          setBase("");
          create.reset();
          // Surface seed problems as a non-blocking note before closing.
          if (view.seed && (view.seed.missing.length > 0 || view.seed.errors.length > 0)) {
            setSeedNote(view.seed);
          } else {
            onClose();
          }
          onCreated(view);
        },
      },
    );
  };

  // A detected default that isn't in the branch list (rare) still needs an
  // option so the Select can show it as selected.
  const options =
    defaultBase && !branches.includes(defaultBase) ? [defaultBase, ...branches] : branches;

  return (
    <Dialog open={open} onClose={close} title="New workstream">
      <form
        className="space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <TextInput
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="What are you working on?"
          autoFocus
          disabled={create.isPending}
        />
        <Field label="Base branch">
          <Select
            value={base}
            onChange={(e) => setBase(e.target.value)}
            disabled={create.isPending || bases.isLoading}
          >
            {bases.isLoading && <option value="">Loading branches…</option>}
            {options.map((b) => (
              <option key={b} value={b}>
                {b}
              </option>
            ))}
          </Select>
        </Field>
        <p className="text-xs leading-relaxed text-ink-dim">
          Creates a dedicated branch and worktree, branched from the base above.
        </p>
        {bases.isError && <ErrorNote error={bases.error} />}
        {create.isError && <ErrorNote error={create.error} />}
        {seedNote && <SeedNote seed={seedNote} />}
        <div className="flex justify-end gap-2">
          <Button type="button" onClick={close} disabled={create.isPending}>
            {seedNote ? "Close" : "Cancel"}
          </Button>
          <Button type="submit" variant="primary" busy={create.isPending} disabled={!title.trim()}>
            {create.isPending ? "Provisioning worktree…" : "Create"}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
