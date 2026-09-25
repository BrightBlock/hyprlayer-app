import { useEffect, useState } from "react";

import type { Project } from "@/lib/bindings";
import { ConfirmDialog } from "@/components/ui";
import { isUserDirectory } from "@/lib/domain/projects";
import { useRemoveProject } from "@/lib/queries";

type Props = {
  target: Project | null;
  onClose: () => void;
  onRemoved: (path: string) => void;
};

export function RemoveProjectDialog({ target, onClose, onRemoved }: Props) {
  const remove = useRemoveProject();
  const [deleteFiles, setDeleteFiles] = useState(false);

  const canDeleteFiles = Boolean(target?.existsOnDisk);
  const userOwned = isUserDirectory(target);

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
    <ConfirmDialog
      open={target !== null}
      onClose={close}
      title="Remove project?"
      confirmLabel={canDeleteFiles && deleteFiles ? "Remove & delete" : "Remove"}
      confirmVariant="danger"
      onConfirm={run}
      busy={remove.isPending}
      error={remove.isError ? remove.error : null}
      onDismissError={() => remove.reset()}
    >
      <p className="text-body leading-prose text-ink-dim">
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
          <span className="min-w-0 text-body">
            <span className="text-ink">Also delete the directory from disk</span>
            <span className="mt-0.5 block truncate text-meta text-ink-dim">
              {target?.path}
            </span>
            <span className={`mt-1 block text-ui ${userOwned ? "text-bad" : "text-ink-dim"}`}>
              {userOwned ? "Your own directory. Can't be undone." : "Permanent."}
            </span>
          </span>
        </label>
      ) : (
        <p className="text-ui text-ink-dim">
          This project's directory is already gone from disk. Only the hyprlayer entry remains to
          remove.
        </p>
      )}
    </ConfirmDialog>
  );
}
