import type { WorkstreamView } from "@/lib/bindings";
import { Button, ConfirmDialog } from "@/components/ui";
import { appOwnsWorktree, isFolderWorkstream } from "@/lib/domain/projects";
import { isAppError } from "@/lib/ipc";
import { useArchiveWorkstream } from "@/lib/queries";

type Props = {
  target: WorkstreamView | null;
  onClose: () => void;
  onArchived: (id: string) => void;
};

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

  const scriptFailed = isAppError(archive.error) && archive.error.code === "archive_script_failed";
  const folder = target !== null && isFolderWorkstream(target);
  const moves = target !== null && appOwnsWorktree(target.item);

  return (
    <ConfirmDialog
      open={target !== null}
      onClose={close}
      title="Archive workstream?"
      confirmLabel={scriptFailed ? "Retry" : "Archive"}
      onConfirm={() => run(false)}
      busy={archive.isPending}
      error={archive.isError ? archive.error : null}
      onDismissError={() => archive.reset()}
      extraActions={
        scriptFailed && (
          <Button type="button" variant="danger" disabled={archive.isPending} onClick={() => run(true)}>
            Skip script &amp; archive
          </Button>
        )
      }
    >
      <p className="text-body leading-prose text-ink-dim">
        {target && (
          <>
            <span className="font-medium text-ink">{target.item.title}</span>
            {!folder && (
              <>
                {" "}
                <span className="text-meta">({target.item.branch})</span>
              </>
            )}{" "}
            :{" "}
          </>
        )}
        {folder ? (
          "the folder is left untouched; only this workstream is archived. You can unarchive it later from the Archive view."
        ) : !moves ? (
          "the folder and branch are left untouched; only this workstream is archived. You can unarchive it later from the Archive view."
        ) : (
          <>
            the worktree moves to <span className="text-meta">archived-contexts/</span>{" "}
            and the branch is preserved. You can unarchive it later from the Archive view.
          </>
        )}
      </p>
    </ConfirmDialog>
  );
}
