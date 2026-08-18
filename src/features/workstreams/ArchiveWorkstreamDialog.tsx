import type { WorkstreamView } from "@/lib/bindings";
import { Button, Dialog, ErrorNote } from "@/components/ui";
import { isFolderWorkstream } from "@/lib/domain/projects";
import { isAppError } from "@/lib/ipc";
import { useArchiveWorkstream } from "@/lib/queries";

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
 * Archive view restores from there. No discard branch, uncommitted work
 * survives by construction.
 */
export function ArchiveWorkstreamDialog({ target, onClose, onArchived }: Props) {
  const archive = useArchiveWorkstream();

  const close = () => {
    if (archive.isPending) return;
    archive.reset();
    onClose();
  };

  const run = (skipScripts: boolean) => {
    if (!target || archive.isPending) return;
    const id = target.item.id;
    archive.mutate(
      { id, skipScripts },
      {
        onSuccess: () => {
          archive.reset();
          onClose();
          onArchived(id);
        },
      },
    );
  };

  // The backend aborts archive (before any teardown) when the project's Archive
  // script fails, so the user can fix it and retry, or skip the script.
  const scriptFailed = isAppError(archive.error) && archive.error.code === "archive_script_failed";
  // Folder rows archive as a pure status flip, no worktree, no branch.
  const folder = target !== null && isFolderWorkstream(target);

  return (
    <Dialog open={target !== null} onClose={close} title="Archive workstream?">
      <div className="space-y-3">
        <p className="text-sm leading-relaxed text-ink-dim">
          {target && (
            <>
              <span className="font-medium text-ink">{target.item.title}</span>
              {!folder && (
                <>
                  {" "}
                  <span className="font-mono text-xs">({target.item.branch})</span>
                </>
              )}{" "}
              :{" "}
            </>
          )}
          {folder ? (
            "the folder is left untouched; only this workstream is archived. You can unarchive it later from the Archive view."
          ) : (
            <>
              the worktree moves to <span className="font-mono text-xs">archived-contexts/</span>{" "}
              and the branch is preserved. You can unarchive it later from the Archive view.
            </>
          )}
        </p>
        {archive.isError && <ErrorNote error={archive.error} />}
        <div className="flex justify-end gap-2">
          <Button type="button" onClick={close} disabled={archive.isPending}>
            Cancel
          </Button>
          {scriptFailed && (
            <Button
              type="button"
              variant="danger"
              disabled={archive.isPending}
              onClick={() => run(true)}
            >
              Skip script &amp; archive
            </Button>
          )}
          <Button type="button" variant="primary" busy={archive.isPending} onClick={() => run(false)}>
            {scriptFailed ? "Retry" : "Archive"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
