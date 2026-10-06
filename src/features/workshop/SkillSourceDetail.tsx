import { useState } from "react";

import { Icon } from "@/components/Icon";
import { Badge, Button, Dialog, ErrorNote, Spinner } from "@/components/ui";
import { MarkdownPreview } from "@/features/library/MarkdownPreview";
import type { SkillInstallPreview, SkillInstallReport, SkillSearchResult } from "@/lib/bindings";
import { formatBytes, plural, shortSha } from "@/lib/format";

import { RailRow, RailSection, RepoLink, SkillDetailFrame } from "./SkillDetailFrame";
import { SkillInstallFlow } from "./SkillInstallFlow";

type Tab = "instructions" | "files";

const noBusy = () => {};

export function SkillSourceDetail({
  result,
  preview,
  error,
  installed,
  projectPath,
  projectName,
  onPreviewAgain,
}: {
  result: SkillSearchResult;
  preview: SkillInstallPreview | null;
  error: unknown;
  installed: boolean;
  projectPath: string | null;
  projectName: string | null;
  onPreviewAgain: () => void;
}) {
  const [tab, setTab] = useState<Tab>("instructions");
  const [adding, setAdding] = useState(false);
  const [report, setReport] = useState<SkillInstallReport | null>(null);
  const tabs = [
    { key: "instructions" as const, label: "Instructions" },
    {
      key: "files" as const,
      label: (
        <>
          Files {preview && <span className="text-ink-dim">{preview.files.length}</span>}
        </>
      ),
    },
  ];

  const header = (
    <div className="flex items-start gap-4">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <h1 className="truncate text-h3 text-ink">{result.name}</h1>
          <Badge tone="dim">skills.sh</Badge>
          {installed && <Badge tone="accent">Installed</Badge>}
          {preview?.flags.scripts && <Badge tone="warn">Runs scripts</Badge>}
        </div>
        <div className="mt-0.5 truncate font-mono text-meta text-ink-dim">{result.source}</div>
        {preview?.description && (
          <p className="mt-1 line-clamp-2 max-w-[640px] text-body leading-prose text-ink-2">{preview.description}</p>
        )}
      </div>
      <Button variant="primary" size="sm" disabled={!preview} onClick={() => setAdding(true)}>
        Add skill
      </Button>
    </div>
  );

  const body =
    error != null ? (
      <div className="space-y-2">
        <ErrorNote error={error} />
        <Button size="sm" onClick={onPreviewAgain}>
          Try again
        </Button>
      </div>
    ) : !preview ? (
      <div className="flex h-24 items-center justify-center">
        <Spinner />
      </div>
    ) : tab === "files" ? (
      <PreviewFiles preview={preview} />
    ) : preview.instructions?.trim() ? (
      <MarkdownPreview body={preview.instructions.trim()} />
    ) : (
      <p className="text-ui text-ink-dim">SKILL.md has no instructions</p>
    );

  return (
    <>
      <SkillDetailFrame
        header={header}
        tabs={tabs}
        tab={tab}
        onTab={setTab}
        body={body}
        rail={<Rail result={result} preview={preview} />}
      />
      {preview && (
        <Dialog
          open={adding}
          onClose={() => setAdding(false)}
          size="lg"
          title="Add skill"
          ariaLabel="Add skill"
        >
          <div className="space-y-3">
            <SkillInstallFlow
              key={preview.token}
              preview={preview}
              projectPath={projectPath}
              projectName={projectName}
              report={report}
              onInstalled={setReport}
              onBusy={noBusy}
              onPreviewAgain={() => {
                setAdding(false);
                onPreviewAgain();
              }}
            />
            {report && (
              <div className="flex justify-end">
                <Button
                  type="button"
                  variant="primary"
                  autoFocus
                  onClick={() => {
                    setAdding(false);
                    setReport(null);
                    // The token is spent; a fresh preview lets the person install it again elsewhere.
                    onPreviewAgain();
                  }}
                >
                  Done
                </Button>
              </div>
            )}
          </div>
        </Dialog>
      )}
    </>
  );
}

function PreviewFiles({ preview }: { preview: SkillInstallPreview }) {
  return (
    <div className="flex flex-col">
      {preview.files.map((f) => {
        const flagged = f.script || f.executable;
        return (
          <div key={f.relPath} className="flex h-8 items-center gap-2.5 border-b border-edge/60 last:border-b-0">
            <span className="flex h-4 w-4 shrink-0 items-center justify-center">
              {flagged ? (
                <Icon name="warning" size="sm" className="text-warn-ink" />
              ) : (
                <Icon name="doc" size="sm" className="text-ink-dim" />
              )}
            </span>
            <span className={`min-w-0 flex-1 truncate font-mono text-meta ${flagged ? "text-warn-ink" : "text-ink"}`}>
              {f.relPath}
            </span>
            <span className="shrink-0 text-meta text-ink-dim">{formatBytes(f.bytes ?? 0)}</span>
          </div>
        );
      })}
    </div>
  );
}

function Rail({ result, preview }: { result: SkillSearchResult; preview: SkillInstallPreview | null }) {
  const bytes = preview?.files.reduce((sum, f) => sum + (f.bytes ?? 0), 0) ?? 0;
  const yesNo = (on: boolean) => (on ? "Yes" : "No");
  return (
    <>
      {result.installs != null && (
        <RailSection title="Popularity">
          <RailRow label="Installs" value={result.installs.toLocaleString()} />
        </RailSection>
      )}
      <RailSection title="Source">
        <RailRow label="Repository" value={<RepoLink repo={result.source} />} mono />
        {preview?.path && <RailRow label="Path" value={preview.path} mono />}
        {preview && (
          <RailRow
            label="Commit"
            mono
            value={
              <>
                {shortSha(preview.commit)} <span className="text-ink-dim">{preview.ref}</span>
              </>
            }
          />
        )}
      </RailSection>
      {preview && (
        <RailSection title="Contents">
          <RailRow label="Bundled" value={`${plural(preview.files.length, "file")} · ${formatBytes(bytes)}`} />
          <RailRow label="Scripts" value={yesNo(preview.flags.scripts)} />
          <RailRow label="Executables" value={yesNo(preview.flags.executables)} />
          <RailRow label="Bash" value={preview.flags.bashAllowed ? "Pre-approved" : "Asks"} />
        </RailSection>
      )}
    </>
  );
}
