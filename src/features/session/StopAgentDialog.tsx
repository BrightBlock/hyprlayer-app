import { ConfirmDialog } from "@/components/ui";

export function StopAgentDialog({
  title,
  onConfirm,
  onClose,
}: {
  title: string | null;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <ConfirmDialog
      open={title !== null}
      onClose={onClose}
      title="Stop agent?"
      confirmLabel="Stop and archive"
      confirmVariant="danger"
      onConfirm={onConfirm}
      autoFocus="confirm"
    >
      <p className="text-body leading-prose text-ink-dim">
        {title} is still working. Archiving it stops the current turn.
      </p>
    </ConfirmDialog>
  );
}
