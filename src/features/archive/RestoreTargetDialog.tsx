import { useId, useState } from "react";

import { PathField } from "@/components/PathField";
import { Button, Dialog, ErrorNote } from "@/components/ui";

import type { ArchiveRow } from "./archiveStatus";

/** Asked when the original path is taken; the path is on the server's machine. */
export function RestoreTargetDialog({
  row,
  busy,
  error,
  onDismissError,
  onClose,
  onRestore,
}: {
  row: ArchiveRow;
  busy: boolean;
  error: unknown;
  onDismissError: () => void;
  onClose: () => void;
  onRestore: (target: string) => void;
}) {
  const formId = useId();
  const original = row.view.item.worktreePath;
  const [path, setPath] = useState(`${original}-restored`);
  const title = `Restore ${row.branch} elsewhere`;
  return (
    <Dialog
      open
      onClose={onClose}
      title={title}
      ariaLabel={title}
      footer={
        <div className="flex justify-end gap-2">
          <Button type="button" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" form={formId} variant="primary" busy={busy} disabled={!path.trim()}>
            Restore
          </Button>
        </div>
      }
    >
      <form
        id={formId}
        className="space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (path.trim()) onRestore(path.trim());
        }}
      >
        <p className="text-sm text-ink-dim">
          <span className="font-mono text-ink-2">{original}</span> is taken. Choose an existing folder
          to restore into, or type a new path.
        </p>
        <PathField value={path} onChange={setPath} browseTitle="Restore into" autoFocus />
        {error != null && <ErrorNote error={error} onDismiss={onDismissError} />}
      </form>
    </Dialog>
  );
}
