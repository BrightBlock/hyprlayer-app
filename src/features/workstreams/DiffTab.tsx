import { DiffView, useDiffMode, type DiffMode } from "@/components/DiffView";
import { ErrorNote, Spinner } from "@/components/ui";
import { useLangForPath } from "@/lib/codeView";
import { shortSha } from "@/lib/format";
import { useCommitFileSides, useFileSides } from "@/lib/queries";
import { renamedLabel } from "./renamedLabel";

export function DiffTab({
  id,
  path,
  staged,
  sha,
}: {
  id: string;
  path: string;
  staged: boolean;
  sha?: string;
}) {
  const working = useFileSides(id, path, staged, !sha);
  const commit = useCommitFileSides(id, sha ?? "", path);
  const sides = sha ? commit : working;
  const [mode, setMode] = useDiffMode();
  const lang = useLangForPath(path);

  const data = sides.data;
  const heading = renamedLabel(path, data?.originalPath ?? null);
  const oneSided = !!data && (!data.originalExists || !data.modifiedExists);
  const effectiveMode: DiffMode = oneSided ? "unified" : mode;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-edge px-4">
        <span className="min-w-0 flex-1 truncate text-meta text-ink-dim" title={heading}>
          {heading}
        </span>
        <span
          className={`shrink-0 rounded bg-edge/60 px-1.5 py-0.5 text-ui-sm font-medium text-ink-dim ${
            sha ? "font-mono" : ""
          }`}
          title={sha}
        >
          {sha ? shortSha(sha) : staged ? "staged" : "working"}
        </span>
        <div className="flex shrink-0 overflow-hidden rounded-md border border-edge">
          {(["unified", "split"] as const).map((m) => (
            <button
              key={m}
              type="button"
              disabled={oneSided}
              onClick={() => setMode(m)}
              className={`px-2 py-0.5 text-ui font-medium disabled:cursor-not-allowed disabled:opacity-40 ${
                effectiveMode === m ? "bg-panel-2 text-ink" : "text-ink-dim hover:bg-panel-2"
              }`}
            >
              {m === "unified" ? "Unified" : "Side by side"}
            </button>
          ))}
        </div>
      </div>
      {sides.isLoading ? (
        <div className="flex flex-1 items-center justify-center">
          <Spinner />
        </div>
      ) : sides.isError ? (
        <div className="p-4">
          <ErrorNote error={sides.error} />
        </div>
      ) : data ? (
        <DiffView original={data.original} modified={data.modified} mode={effectiveMode} lang={lang} />
      ) : null}
    </div>
  );
}
