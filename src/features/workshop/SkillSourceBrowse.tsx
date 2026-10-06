import { useMemo, useState, type ReactNode } from "react";

import { Badge, Button, EmptyState, ErrorNote, SegmentedControl, Spinner, TextInput } from "@/components/ui";
import { ListCardRow, ListHeader, WorkshopFrame } from "@/components/workshop";
import type { SkillFile, SkillSearchResult, SkillsShView } from "@/lib/bindings";
import { formatCompactCount } from "@/lib/format";
import { usePreviewSkillInstall, useSkillLeaderboard, useSkillSourceSearch } from "@/lib/queries";
import { useDebounced } from "@/lib/useDebounced";

import { SkillSourceDetail } from "./SkillSourceDetail";
import { SKILLS_SH_VIEWS, skillFolder } from "./skills";

const SEARCH_DEBOUNCE_MS = 300;

const sameResult = (a: SkillSearchResult, b: SkillSearchResult) => a.source === b.source && a.skillId === b.skillId;


export function SkillSourceBrowse({
  nav,
  skills,
  projectPath,
  projectName,
}: {
  nav: ReactNode;
  skills: SkillFile[];
  projectPath: string | null;
  projectName: string | null;
}) {
  const [query, setQuery] = useState("");
  const [pick, setPick] = useState<SkillSearchResult | null>(null);
  const debounced = useDebounced(query.trim(), SEARCH_DEBOUNCE_MS);
  const results = useSkillSourceSearch(debounced);
  const [view, setView] = useState<SkillsShView>("all-time");
  const searching = debounced.length >= 2;
  const board = useSkillLeaderboard(view, !searching);
  const popular = useMemo(() => {
    const seen = new Set<string>();
    return (board.data?.pages ?? [])
      .flatMap((p) => p.skills)
      .filter((r) => {
        const key = `${r.source}/${r.skillId}`;
        return !seen.has(key) && (seen.add(key), true);
      });
  }, [board.data]);
  const preview = usePreviewSkillInstall();

  const choose = (result: SkillSearchResult) => {
    setPick(result);
    preview.mutate({
      projectPath,
      source: { kind: "skillsSh", source: result.source, skillId: result.skillId },
    });
  };
  const installed = (r: SkillSearchResult) =>
    skills.some((s) => s.origin?.repo === r.source && skillFolder(s) === r.skillId);
  const rows = searching ? (results.data ?? []) : popular;

  return (
    <WorkshopFrame
      navKey="skills:view:skills.sh"
      nav={nav}
      list={
        <>
          <ListHeader>skills.sh</ListHeader>
          <div className="shrink-0 border-b border-edge p-3">
            <TextInput
              autoFocus
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search…"
              aria-label="Search skills.sh"
            />
            {!searching && (
              <SegmentedControl
                className="mt-2 w-full [&>button]:flex-1"
                aria-label="Leaderboard"
                value={view}
                options={SKILLS_SH_VIEWS}
                onChange={setView}
              />
            )}
          </div>
          <div className="min-h-0 flex-1 space-y-0.5 overflow-y-auto px-2 pb-3 pt-2">
            {searching && results.isError && <ErrorNote error={results.error} />}
            {!searching && board.isError && <ErrorNote error={board.error} />}
            {!searching && board.isLoading && (
              <div className="flex h-24 items-center justify-center">
                <Spinner />
              </div>
            )}
            {rows.map((r) => (
              <ListCardRow
                key={`${r.source}/${r.skillId}`}
                title={r.name}
                badges={installed(r) ? <Badge tone="dim">Installed</Badge> : undefined}
                meta={r.source}
                trailing={r.installs == null ? undefined : formatCompactCount(r.installs)}
                selected={pick != null && sameResult(pick, r)}
                onClick={() => choose(r)}
              />
            ))}
            {rows.length === 0 && searching && !results.isError && !results.isFetching && (
              <div className="mx-0.5 rounded-md border border-dashed border-edge px-2.5 py-2.5 text-ui leading-prose text-ink-dim">
                {`No skills match “${debounced}”`}
              </div>
            )}
            {!searching && board.hasNextPage && (
              <Button
                size="sm"
                className="mt-2 w-full justify-center"
                busy={board.isFetchingNextPage}
                onClick={() => void board.fetchNextPage()}
              >
                Load more
              </Button>
            )}
          </div>
        </>
      }
      detail={
        pick == null ? (
          <EmptyState title="Select a skill" />
        ) : (
          <SkillSourceDetail
            key={`${pick.source}/${pick.skillId}`}
            result={pick}
            preview={preview.isPending ? null : (preview.data ?? null)}
            error={preview.isError ? preview.error : null}
            installed={installed(pick)}
            projectPath={projectPath}
            projectName={projectName}
            onPreviewAgain={() => choose(pick)}
          />
        )
      }
    />
  );
}
