import { useNavigate, useSearch } from "@tanstack/react-router";

import { LibraryView } from "@/features/library/LibraryView";

export function LibraryPage() {
  const search = useSearch({ from: "/library" });
  const navigate = useNavigate({ from: "/library" });
  return (
    <LibraryView
      search={search}
      onUpdate={(patch) => void navigate({ search: (prev) => ({ ...prev, ...patch }), replace: true })}
    />
  );
}
