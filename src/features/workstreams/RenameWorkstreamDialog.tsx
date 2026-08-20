import { useEffect, useState } from "react";

import type { WorkstreamView } from "@/lib/bindings";
import { Button, Dialog, ErrorNote, TextInput } from "@/components/ui";
import { useRenameWorkstream } from "@/lib/queries";

type Props = {
  /** The workstream being renamed; null keeps the dialog closed. */
  target: WorkstreamView | null;
  onClose: () => void;
};

export function RenameWorkstreamDialog({ target, onClose }: Props) {
  const rename = useRenameWorkstream();
  const [value, setValue] = useState("");

  useEffect(() => {
    if (target) setValue(target.item.title);
  }, [target]);

  const close = () => {
    if (rename.isPending) return;
    rename.reset();
    onClose();
  };

  const submit = () => {
    if (!target || rename.isPending) return;
    const title = value.trim();
    if (!title) return;
    rename.mutate({ id: target.item.id, title }, { onSuccess: close });
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
