import { useState } from "react";

import { Button, Dialog } from "@/components/ui";
import type { SkillInstallPreview, SkillInstallReport, SkillScope } from "@/lib/bindings";

import { SkillInstallFlow } from "./SkillInstallFlow";

type Props = {
  preview: SkillInstallPreview | null;
  scope: SkillScope;
  restoring?: boolean;
  projectPath: string | null;
  projectName: string | null;
  onDone: () => void;
  onCancel: () => void;
};

// Re-approval for an update or a restore whose content changed.
export function SkillReviewDialog({ preview, ...rest }: Props) {
  return preview ? <ReviewBody key={preview.token} preview={preview} {...rest} /> : null;
}

function ReviewBody({
  preview,
  scope,
  restoring,
  projectPath,
  projectName,
  onDone,
  onCancel,
}: Props & { preview: SkillInstallPreview }) {
  const [report, setReport] = useState<SkillInstallReport | null>(null);
  const [busy, setBusy] = useState(false);
  const title = restoring ? "Restore skill" : "Update skill";

  return (
    <Dialog open onClose={() => !busy && (report ? onDone() : onCancel())} size="lg" title={title} ariaLabel={title}>
      <div className="space-y-3">
        <SkillInstallFlow
          preview={preview}
          projectPath={projectPath}
          projectName={projectName}
          lockedScope={scope}
          restoring={restoring}
          report={report}
          onInstalled={setReport}
          onBusy={setBusy}
          onPreviewAgain={onCancel}
        />
        {report && (
          <div className="flex justify-end">
            <Button type="button" variant="primary" autoFocus onClick={onDone}>
              Done
            </Button>
          </div>
        )}
      </div>
    </Dialog>
  );
}
