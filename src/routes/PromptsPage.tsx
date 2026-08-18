import { useNavigate, useSearch } from "@tanstack/react-router";

import { PageError, PageSpinner } from "@/components/ui";
import { PromptLibrary } from "@/features/prompts/PromptLibrary";
import { useBootstrap } from "@/lib/queries";

/**
 * The Prompt library route (`/prompts`), opened from the nav rail's ⚡ entry,
 * a master–detail view like the Library: the prompts panel beside the selected
 * prompt's editor. The selected prompt id lives in the URL (`?prompt=`) so a
 * view is shareable/restorable; the default selection (Commit) keeps a bare
 * URL. The bootstrap gate matters here: `PromptLibrary` seeds its local draft
 * from settings ONCE on mount, so it must not mount before settings exist.
 */
export function PromptsPage() {
  const search = useSearch({ from: "/prompts" });
  const navigate = useNavigate({ from: "/prompts" });
  const bootstrap = useBootstrap();

  if (bootstrap.isLoading) return <PageSpinner />;
  if (bootstrap.isError) return <PageError error={bootstrap.error} />;

  return (
    <PromptLibrary
      settings={bootstrap.data!.settings}
      selectedId={search.prompt ?? "commit"}
      onSelect={(id) =>
        void navigate({
          search: { prompt: id === "commit" ? undefined : id },
          replace: true,
        })
      }
    />
  );
}
