import type { WorkstreamView } from "../../lib/bindings";
import { Button, Dialog, ErrorNote } from "../../components/ui";
import { isAppError } from "../../lib/ipc";
import { useArchiveWorkstream } from "../../lib/queries";

type Props = {
  /** The item being archived; null keeps the dialog closed. */
  target: WorkstreamView | null;
  onClose: () => void;
  /** Called after a successful archive so the caller can fix tab selection. */
  onArchived: (id: string) => void;
};

const BLOCKING_CODES = new Set(["worktree_dirty", "worktree_unpushed"]);

/**
 * Two-step archive: the safe call (discard=false) first; if the backend
 * refuses because work would be lost, surface its message + remedy and offer
 * the destructive retry (discard=true).
 */
export function ArchiveWorkstreamDialog({ target, onClose, onArchived }: Props) {
  const archive = useArchiveWorkstream();

  const blocked = archive.isError && isAppError(archive.error) && BLOCKING_CODES.has(archive.error.code);

  const close = () => {
    if (archive.isPending) return;
    archive.reset();
    onClose();
  };

  const run = (discard: boolean) => {
    if (!target || archive.isPending) return;
    const id = target.item.id;
    archive.mutate(
      { id, discard },
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
          the worktree and branch will be removed.
        </p>
        {archive.isError && <ErrorNote error={archive.error} />}
        <div className="flex justify-end gap-2">
          <Button type="button" onClick={close} disabled={archive.isPending}>
            Cancel
          </Button>
          {blocked ? (
            <Button
              type="button"
              variant="danger"
              busy={archive.isPending}
              onClick={() => run(true)}
            >
              Discard changes & archive
            </Button>
          ) : (
            <Button
              type="button"
              variant="primary"
              busy={archive.isPending}
              onClick={() => run(false)}
            >
              Archive
            </Button>
          )}
        </div>
      </div>
    </Dialog>
  );
}
