import { useEffect, useState } from "react";

import type { BranchRenameBlock, WorkstreamView } from "@/lib/bindings";
import { Button, Dialog, ErrorNote, TextInput, Toggle } from "@/components/ui";
import { useBranchRenamePreview, useRenameWorkstream } from "@/lib/queries";
import { useDebounced } from "@/lib/useDebounced";

type Props = {
  target: WorkstreamView | null;
  onClose: () => void;
};

const BLOCKED: Record<BranchRenameBlock, string> = {
  pushed: "Already pushed",
  hasPr: "Has a pull request",
  moved: "The worktree is on another branch",
  taken: "A branch with this name exists",
};

export function RenameWorkstreamDialog({ target, onClose }: Props) {
  const rename = useRenameWorkstream();
  const [value, setValue] = useState("");
  const [renameBranch, setRenameBranch] = useState(true);
  const title = useDebounced(value.trim(), 200);
  const changed = Boolean(target) && title !== target?.item.title;
  const preview = useBranchRenamePreview(target?.item.id ?? "", title, changed).data ?? null;
  const offered = changed ? preview : null;
  const blocked = offered?.blocked ?? null;

  useEffect(() => {
    if (target) {
      setValue(target.item.title);
      setRenameBranch(true);
    }
  }, [target]);

  const close = () => {
    if (rename.isPending) return;
    rename.reset();
    onClose();
  };

  const submit = () => {
    if (!target || rename.isPending) return;
    const next = value.trim();
    if (!next) return;
    const branch = Boolean(offered) && !blocked && renameBranch;
    rename.mutate({ id: target.item.id, title: next, branch }, { onSuccess: close });
  };

  return (
    <Dialog open={Boolean(target)} onClose={close} title="Rename workstream">
      <form
        className="space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <TextInput
          value={value}
          onChange={(e) => setValue(e.target.value)}
          autoFocus
          disabled={rename.isPending}
        />
        {offered && (
          <div title={blocked ? BLOCKED[blocked] : undefined}>
            <Toggle
              checked={renameBranch && !blocked}
              onChange={setRenameBranch}
              disabled={Boolean(blocked) || rename.isPending}
            >
              <span className="min-w-0 text-ui text-ink-dim">
                Rename branch to <span className="font-mono text-ink">{offered.branch}</span>
              </span>
            </Toggle>
          </div>
        )}
        {rename.isError && <ErrorNote error={rename.error} onDismiss={() => rename.reset()} />}
        <div className="flex justify-end gap-2">
          <Button type="button" onClick={close} disabled={rename.isPending}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" busy={rename.isPending} disabled={!value.trim()}>
            Rename
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
