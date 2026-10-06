import { useSearch } from "@tanstack/react-router";

import { useBootstrap } from "@/lib/queries";
import { useLibraryScope } from "./libraryScope";
import { resolveSyncProjectPath } from "./thoughtsSync";
import { ThoughtsSyncButton } from "./ThoughtsSyncButton";
import { useThoughtsSync } from "./useThoughtsSync";

// The Library's only useThoughtsSync caller, so auto-sync fires once per visit.
export function LibrarySyncButton() {
  // The header can render a frame before the lazy /library match resolves; resolving then
  // would pick the last scope, not this visit's, and auto-sync it.
  const search = useSearch({ from: "/library", shouldThrow: false });
  const lastProject = useLibraryScope((s) => s.lastProject);
  const projects = useBootstrap().data?.projects ?? [];
  const sync = useThoughtsSync(search && resolveSyncProjectPath(search, lastProject, projects));
  return <ThoughtsSyncButton sync={sync} />;
}
