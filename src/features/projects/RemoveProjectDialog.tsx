import { useEffect, useState } from "react";

import type { Project } from "../../lib/bindings";
import { Button, Dialog, ErrorNote } from "../../components/ui";
import { useRemoveProject } from "../../lib/queries";

type Props = {
  /** The project being removed; null keeps the dialog closed. */
  target: Project | null;
  onClose: () => void;
  /** Called after a successful removal so the caller can fix navigation. */
  onRemoved: (path: string) => void;
};

/**
 * Remove a project from the sidebar. Always drops it from the registry; the
 * checkbox additionally deletes the directory from disk. For `disk`-sourced
 * projects that directory is the user's own working copy, so we warn harder.
 */
export function RemoveProjectDialog({ target, onClose, onRemoved }: Props) {
  const remove = useRemoveProject();
  const [deleteFiles, setDeleteFiles] = useState(false);

  // Anything still on disk can be deleted (backend refuses while it has active
  // workstreams). An in-place import is the user's own working copy, not an
  // app-owned clone, so the warning is sterner.
  const canDeleteFiles = Boolean(target?.existsOnDisk);
  const isImported = target?.source === "disk";

  // Reset the toggle whenever a different project opens the dialog.
  useEffect(() => {
    setDeleteFiles(false);
  }, [target?.path]);

  const close = () => {
    if (remove.isPending) return;
    remove.reset();
    onClose();
  };

  const run = () => {
    if (!target || remove.isPending) return;
    const path = target.path;
    remove.mutate(
      { path, deleteFiles: canDeleteFiles && deleteFiles },
      {
        onSuccess: () => {
          remove.reset();
          onClose();
          onRemoved(path);
        },
      },
    );
  };

  return (
    <Dialog open={target !== null} onClose={close} title="Remove project?">
      <div className="space-y-3">
        <p className="text-sm leading-relaxed text-ink-dim">
          {target && <span className="font-medium text-ink">{target.mappedName}</span>} will be
          removed from the sidebar. Its workstreams stop showing in hyprlayer.
        </p>

        {canDeleteFiles ? (
          <label className="flex cursor-pointer items-start gap-2.5 rounded-md border border-edge bg-panel-2 p-3">
            <input
              type="checkbox"
              className="mt-0.5 h-4 w-4 shrink-0 cursor-pointer accent-accent"
              checked={deleteFiles}
              onChange={(e) => setDeleteFiles(e.target.checked)}
              disabled={remove.isPending}
            />
            <span className="min-w-0 text-sm">
              <span className="text-ink">Also delete the directory from disk</span>
              <span className="mt-0.5 block truncate font-mono text-xs text-ink-dim">
                {target?.path}
              </span>
              <span className={`mt-1 block text-xs ${isImported ? "text-bad" : "text-ink-dim"}`}>
                {isImported ? "Your working copy — can't be undone." : "Permanent."}
              </span>
            </span>
          </label>
        ) : (
          <p className="text-xs text-ink-dim">
            This project's directory is already gone from disk — only the hyprlayer entry remains
            to remove.
          </p>
        )}

        {remove.isError && <ErrorNote error={remove.error} />}

        <div className="flex justify-end gap-2">
          <Button type="button" onClick={close} disabled={remove.isPending}>
            Cancel
          </Button>
          <Button type="button" variant="danger" busy={remove.isPending} onClick={run}>
            {canDeleteFiles && deleteFiles ? "Remove & delete" : "Remove"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
