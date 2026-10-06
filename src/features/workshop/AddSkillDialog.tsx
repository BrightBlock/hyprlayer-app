import { useState } from "react";

import { Badge, Button, Dialog, ErrorNote, TextInput } from "@/components/ui";
import type { SkillInstallPreview, SkillInstallReport, SkillScope } from "@/lib/bindings";
import { usePreviewSkillInstall, useSkillfishCandidates } from "@/lib/queries";

import { SkillInstallFlow } from "./SkillInstallFlow";

export type SkillAdded = { folder: string; scope: SkillScope };

type Props = {
  open: boolean;
  onClose: () => void;
  onDone: (added: SkillAdded) => void;
  projectPath: string | null;
  projectName: string | null;
};

export function AddSkillDialog({ open, ...rest }: Props) {
  return open ? <AddSkillBody {...rest} /> : null;
}

function AddSkillBody({ onClose, onDone, projectPath, projectName }: Omit<Props, "open">) {
  const [url, setUrl] = useState("");
  const [preview, setPreview] = useState<SkillInstallPreview | null>(null);
  const [report, setReport] = useState<SkillInstallReport | null>(null);
  const [installing, setInstalling] = useState(false);
  const [importing, setImporting] = useState(false);
  const fetchPreview = usePreviewSkillInstall();
  const candidates = (useSkillfishCandidates(projectPath, preview == null).data ?? []).filter((c) => !c.installed);
  const busy = fetchPreview.isPending || installing;
  const link = url.trim();

  const runPreview = (spec: string) => {
    if (!spec || busy) return;
    fetchPreview.mutate({ projectPath, source: { kind: "url", url: spec } }, { onSuccess: setPreview });
  };

  return (
    <Dialog open onClose={() => !busy && onClose()} size="lg" title="Add skill" ariaLabel="Add skill">
      {preview ? (
        <div className="space-y-3">
          <SkillInstallFlow
            preview={preview}
            projectPath={projectPath}
            projectName={projectName}
            report={report}
            onInstalled={setReport}
            onBusy={setInstalling}
            onPreviewAgain={() => {
              setPreview(null);
              fetchPreview.reset();
            }}
          />
          {report && (
            <div className="flex justify-end">
              <Button
                type="button"
                variant="primary"
                autoFocus
                onClick={() => {
                  onDone({ folder: preview.name, scope: report.scope });
                  onClose();
                }}
              >
                Done
              </Button>
            </div>
          )}
        </div>
      ) : (
        <form
          className="space-y-3"
          onSubmit={(e) => {
            e.preventDefault();
            runPreview(link);
          }}
        >
          <TextInput
            autoFocus
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="github.com/owner/repo/tree/main/skills/name"
            aria-label="Skill link"
          />
          {importing && (
            <div className="space-y-0.5">
              {candidates.map((c) => (
                <div key={`${c.scope}:${c.spec}`} className="flex h-8 items-center gap-2.5 px-1">
                  <span className="min-w-0 flex-1 truncate font-mono text-meta text-ink">{c.spec}</span>
                  <Badge tone="dim">{c.scope === "project" ? "This project" : "Global"}</Badge>
                  <Button type="button" size="sm" disabled={busy} onClick={() => runPreview(c.spec)}>
                    Preview
                  </Button>
                </div>
              ))}
            </div>
          )}
          {fetchPreview.isError && <ErrorNote error={fetchPreview.error} />}
          <div className="flex items-center justify-between">
            {candidates.length > 0 ? (
              <Button type="button" variant="ghost" onClick={() => setImporting((on) => !on)}>
                {`Import from skillfish.json (${candidates.length})`}
              </Button>
            ) : (
              <span />
            )}
            <Button type="submit" variant="primary" busy={fetchPreview.isPending} disabled={!link}>
              Preview
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}
