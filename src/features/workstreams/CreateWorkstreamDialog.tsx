import { useState } from "react";

import type { WorkstreamView } from "../../lib/bindings";
import { Button, Dialog, ErrorNote, TextInput } from "../../components/ui";
import { useCreateWorkstream } from "../../lib/queries";

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
 */
export function CreateWorkstreamDialog({ projectPath, open, onClose, onCreated }: Props) {
  const [title, setTitle] = useState("");
  const create = useCreateWorkstream(projectPath);

  const close = () => {
    if (create.isPending) return;
    setTitle("");
    create.reset();
    onClose();
  };

  const submit = () => {
    const trimmed = title.trim();
    if (!trimmed || create.isPending) return;
    create.mutate(trimmed, {
      onSuccess: (view) => {
        setTitle("");
        create.reset();
        onClose();
        onCreated(view);
      },
    });
  };

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
        <p className="text-xs leading-relaxed text-ink-dim">
          Creates a dedicated branch and worktree for this item.
        </p>
        {create.isError && <ErrorNote error={create.error} />}
        <div className="flex justify-end gap-2">
          <Button type="button" onClick={close} disabled={create.isPending}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" busy={create.isPending} disabled={!title.trim()}>
            {create.isPending ? "Provisioning worktree…" : "Create"}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
