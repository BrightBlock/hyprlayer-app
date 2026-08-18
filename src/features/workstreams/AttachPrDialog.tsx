import { useEffect, useState } from "react";

import type { WorkstreamView } from "@/lib/bindings";
import { Dialog, ErrorNote, Spinner, TextInput } from "@/components/ui";
import { GitPullRequestIcon } from "@/components/icons";
import { useAttachPr, useWorkstreamOpenPrs, useWorkstreamPrPreview } from "@/lib/queries";

type Props = {
  /** The workstream to attach a PR to; null keeps the dialog closed. */
  target: WorkstreamView | null;
  onClose: () => void;
};

/** Parse a bare PR number (`123` or `#123`) from input; null for anything else. */
function parsePrNumber(s: string): number | null {
  const t = s.trim();
  return /^#?\d+$/.test(t) ? Number(t.replace(/^#/, "")) : null;
}

/**
 * The "Attach PR" picker (from the workstream context menu). Stays empty until
 * the user types: a PR number resolves to a single live preview (confirming it
 * exists before attaching), while free text searches the repo's open PRs. We
 * deliberately don't dump every open PR on open, that list can get long.
 */
export function AttachPrDialog({ target, onClose }: Props) {
  const id = target?.item.id ?? "";
  const [input, setInput] = useState("");
  const [query, setQuery] = useState("");
  const attach = useAttachPr(id);

  // Reset to a clean slate each time the dialog opens for a new workstream.
  useEffect(() => {
    if (target) {
      setInput("");
      setQuery("");
      attach.reset();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target]);

  // Debounce the fetch so each keystroke doesn't spawn a `gh` subprocess.
  useEffect(() => {
    const t = setTimeout(() => setQuery(input.trim()), 250);
    return () => clearTimeout(t);
  }, [input]);

  const open = Boolean(target);
  const queryNumber = parsePrNumber(query);
  const inputNumber = parsePrNumber(input);
  const isSearch = query.length > 0 && queryNumber === null;

  // Number → single preview; text → scoped search. Only one runs at a time.
  const preview = useWorkstreamPrPreview(id, queryNumber, open);
  const search = useWorkstreamOpenPrs(id, query, open && isSearch);

  const close = () => {
    if (attach.isPending) return;
    onClose();
  };

  const choose = (prNumber: number) => {
    if (attach.isPending) return;
    attach.mutate(prNumber, { onSuccess: close });
  };

  const searchRows = search.data ?? [];

  return (
    <Dialog open={open} onClose={close} title="Attach PR">
      <div className="space-y-3">
        <TextInput
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="Search PRs or enter a number…"
          autoFocus
          disabled={attach.isPending}
          onKeyDown={(e) => {
            // Enter on a bare number attaches it straight away.
            if (e.key === "Enter" && inputNumber !== null) {
              e.preventDefault();
              choose(inputNumber);
            }
          }}
        />

        {attach.isError && <ErrorNote error={attach.error} />}

        <div className="max-h-72 overflow-y-auto">
          {query.length === 0 ? (
            <p className="py-6 text-center text-sm text-ink-dim">
              Search by title, or enter a PR number.
            </p>
          ) : queryNumber !== null ? (
            // Number entered → preview that exact PR.
            preview.isLoading ? (
              <div className="flex justify-center py-6">
                <Spinner size={16} />
              </div>
            ) : preview.data ? (
              <PrRow
                title={preview.data.title}
                subtitle={`#${preview.data.number} · ${preview.data.headRefName}${
                  preview.data.isDraft ? " · draft" : ""
                }`}
                busy={attach.isPending}
                onClick={() => choose(preview.data!.number)}
              />
            ) : (
              <p className="py-6 text-center text-sm text-ink-dim">No PR #{queryNumber} found</p>
            )
          ) : // Free text → scoped search of the repo's open PRs.
          search.isLoading ? (
            <div className="flex justify-center py-6">
              <Spinner size={16} />
            </div>
          ) : searchRows.length === 0 ? (
            <p className="py-6 text-center text-sm text-ink-dim">No open PRs found</p>
          ) : (
            searchRows.map((pr) => (
              <PrRow
                key={pr.number}
                title={pr.title}
                subtitle={`#${pr.number} · ${pr.headRefName}${pr.isDraft ? " · draft" : ""}`}
                busy={attach.isPending}
                onClick={() => choose(pr.number)}
              />
            ))
          )}
        </div>
      </div>
    </Dialog>
  );
}

function PrRow({
  title,
  subtitle,
  busy,
  onClick,
}: {
  title: string;
  subtitle: string;
  busy: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy}
      className="flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-sm text-ink-dim hover:bg-edge/60 hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
    >
      <GitPullRequestIcon size={14} className="mt-0.5 shrink-0 self-start" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-ink">{title}</span>
        <span className="block truncate text-xs text-ink-dim">{subtitle}</span>
      </span>
    </button>
  );
}
