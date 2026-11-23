import { useState } from "react";

import type { WorkstreamView } from "@/lib/bindings";
import { Button, ConfirmDialog } from "@/components/ui";
import { appOwnsWorktree, isFolderWorkstream } from "@/lib/domain/projects";
import { isAppError } from "@/lib/ipc";
import { useArchiveWorkstream, useChildWorkstreams } from "@/lib/queries";

type Props = {
  target: WorkstreamView | null;
  onClose: () => void;
  onArchived: (id: string) => void;
};

export function ArchiveWorkstreamDialog({ target, onClose, onArchived }: Props) {
  const archive = useArchiveWorkstream();
  const children = useChildWorkstreams(target?.item.id ?? null).data ?? [];
  const [withChildren, setWithChildren] = useState(false);

  const close = () => {
    if (archive.isPending) return;
    archive.reset();
    setWithChildren(false);
    onClose();
  };

  const run = (skipScripts: boolean) => {
    if (!target || archive.isPending) return;
    const id = target.item.id;
    const childIds = withChildren ? children.map((c) => c.id) : [];
    archive.mutate(
      { id, skipScripts, childIds },
      {
        onSuccess: () => {
          archive.reset();
          setWithChildren(false);
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
      {children.length > 0 && (
        <div className="flex flex-col gap-1.5">
          <p className="text-body leading-prose text-ink-dim">Its agents made these workstreams, which are still active:</p>
          <ul className="flex flex-col gap-1 text-ui">
            {children.map((child) => (
              <li key={child.id}>
                <span className="font-medium text-ink">{child.title}</span>{" "}
                <span className="text-ink-dim">
                  {child.changedFiles.length === 0
                    ? "· no uncommitted changes"
                    : `· ${child.changedFiles.length} changed: ${child.changedFiles.join(", ")}`}
                </span>
              </li>
            ))}
          </ul>
          <label className="flex cursor-pointer items-center gap-1.5 text-ui text-ink-dim hover:text-ink">
            <input
              type="checkbox"
              className="h-3 w-3 shrink-0 cursor-pointer accent-accent"
              checked={withChildren}
              onChange={(e) => setWithChildren(e.target.checked)}
              disabled={archive.isPending}
            />
            <span>
              Archive {children.length === 1 ? "it" : `all ${children.length}`} too; uncommitted changes move to{" "}
              <span className="text-meta">archived-contexts/</span> with each worktree
            </span>
          </label>
        </div>
      )}
    </ConfirmDialog>
  );
}
