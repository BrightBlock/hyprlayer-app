import type { WorkstreamView } from "../../lib/bindings";
import { Button, Dialog, ErrorNote } from "../../components/ui";
import { useArchiveWorkstream } from "../../lib/queries";

type Props = {
  /** The item being archived; null keeps the dialog closed. */
  target: WorkstreamView | null;
  onClose: () => void;
  /** Called after a successful archive so the caller can fix tab selection. */
  onArchived: (id: string) => void;
};

/**
 * Non-destructive archive: the worktree is moved (not deleted) to
 * `<project>/archived-contexts/<slug>/` and the branch is preserved. The
 * Archive view restores from there. No discard branch — uncommitted work
 * survives by construction.
 */
export function ArchiveWorkstreamDialog({ target, onClose, onArchived }: Props) {
  const archive = useArchiveWorkstream();

  const close = () => {
    if (archive.isPending) return;
    archive.reset();
    onClose();
  };

  const run = () => {
    if (!target || archive.isPending) return;
    const id = target.item.id;
    archive.mutate(
      { id },
      {
        onSuccess: () => {
          archive.reset();
          onClose();
          onArchived(id);
        },
      },
    );
  };

  return (
    <Dialog open={target !== null} onClose={close} title="Archive workstream?">
      <div className="space-y-3">
        <p className="text-sm leading-relaxed text-ink-dim">
          {target && (
            <>
              <span className="font-medium text-ink">{target.item.title}</span>{" "}
              <span className="font-mono text-xs">({target.item.branch})</span> —{" "}
            </>
          )}
          the worktree moves to <span className="font-mono text-xs">archived-contexts/</span> and
          the branch is preserved. You can unarchive it later from the Archive view.
        </p>
        {archive.isError && <ErrorNote error={archive.error} />}
        <div className="flex justify-end gap-2">
          <Button type="button" onClick={close} disabled={archive.isPending}>
            Cancel
          </Button>
          <Button
            type="button"
            variant="primary"
            busy={archive.isPending}
            onClick={run}
          >
            Archive
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
