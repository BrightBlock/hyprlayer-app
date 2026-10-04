import { useMemo, useState } from "react";

import { Badge, EmptyState, ErrorNote, PageSpinner, TextInput } from "@/components/ui";
import {
  ListCardRow,
  ListHeader,
  NavFilterRow,
  NavGroupLabel,
  WorkshopFrame,
  WorkshopNav,
  type WorkshopChrome,
} from "@/components/workshop";
import { plural } from "@/lib/format";
import { useAgentFiles, useSkillFiles } from "@/lib/queries";

import { SkillDetail } from "./SkillDetail";
import {
  attachedAgents,
  filterSkills,
  SKILL_FILTERS,
  skillFilter,
  skillMeta,
  skillSourceLabel,
  type SkillFilter,
  type SkillTab,
} from "./skills";

export function SkillLibrary({
  chrome,
  projectPath,
  projectName,
  selectedPath,
  onSelect,
  filter: rawFilter,
  onFilter,
  onOpenAgent,
  tab,
  onTab,
}: {
  chrome: WorkshopChrome;
  projectPath: string | null;
  projectName: string | null;
  selectedPath: string | null;
  onSelect: (path: string | null) => void;
  filter: string | undefined;
  onFilter: (filter: SkillFilter) => void;
  onOpenAgent: (path: string, target: "agents" | "system") => void;
  tab: SkillTab;
  onTab: (tab: SkillTab) => void;
}) {
  const skills = useSkillFiles(projectPath);
  const agents = useAgentFiles(projectPath);
  const [search, setSearch] = useState("");
  const filter = skillFilter(rawFilter);

  const all = useMemo(() => skills.data ?? [], [skills.data]);
  const inFilter = useMemo(() => filterSkills(all, filter), [all, filter]);
  const visible = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return inFilter;
    return inFilter.filter(
      (s) => s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q),
    );
  }, [inFilter, search]);

  const selected = selectedPath ? (all.find((s) => s.path === selectedPath) ?? null) : null;
  const filterLabel = SKILL_FILTERS.find((f) => f.key === filter)?.label ?? "All";

  if (skills.isLoading) return <PageSpinner />;

  return (
    <WorkshopFrame
      navKey={`skills:${filter}`}
      nav={
        <WorkshopNav
          chrome={chrome}
          filters={
            <div>
              <NavGroupLabel>Source</NavGroupLabel>
              <div className="space-y-0.5">
                {SKILL_FILTERS.map((f) => (
                  <NavFilterRow
                    key={f.key}
                    label={f.label}
                    count={filterSkills(all, f.key).length}
                    selected={filter === f.key}
                    onClick={() => onFilter(f.key)}
                  />
                ))}
              </div>
            </div>
          }
        />
      }
      list={
        <>
          <ListHeader>{`${filterLabel} · ${plural(visible.length, "skill")}`}</ListHeader>
          <div className="shrink-0 border-b border-edge p-3">
            <TextInput
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Filter…"
              aria-label="Filter skills"
            />
          </div>
          <div className="min-h-0 flex-1 space-y-0.5 overflow-y-auto px-2 pb-3 pt-2">
            {skills.isError && <ErrorNote error={skills.error} />}
            {visible.map((s) => (
              <ListCardRow
                key={s.path}
                title={s.name}
                badges={
                  <>
                    <Badge tone="dim">{skillSourceLabel(s)}</Badge>
                    {s.userOnly && <Badge tone="dim">User-only</Badge>}
                  </>
                }
                description={s.description}
                meta={skillMeta(s)}
                selected={s.path === selectedPath}
                onClick={() => onSelect(s.path)}
              />
            ))}
            {visible.length === 0 && !skills.isError && !skills.isFetching && (
              <div className="mx-0.5 rounded-md border border-dashed border-edge px-2.5 py-2.5 text-ui leading-prose text-ink-dim">
                {search.trim() ? `No skills match “${search.trim()}”` : "No skills"}
              </div>
            )}
          </div>
        </>
      }
      detail={
        selected ? (
          <SkillDetail
            skill={selected}
            projectPath={projectPath}
            projectName={projectName}
            attached={attachedAgents(selected, agents.data ?? [])}
            onOpenAgent={onOpenAgent}
            tab={tab}
            onTab={onTab}
          />
        ) : (
          <EmptyState title="Select a skill" />
        )
      }
    />
  );
}
