import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import type { ChecksState, WorkstreamView } from "../../lib/bindings";
import { useWorkstreamPr } from "../../lib/queries";
import { Spinner } from "../../components/ui";
import { ChevronDownIcon, ChevronRightIcon } from "../../components/icons";

/** Overall-status dot for the CI Checks header: green passing, red failing,
 *  amber (pulsing) pending, dim when there are no checks. */
function StatusDot({ state }: { state: ChecksState }) {
  const tone =
    state === "passing"
      ? "bg-ok"
      : state === "failing"
        ? "bg-bad"
        : state === "pending"
          ? "bg-warn animate-pulse"
          : "bg-ink-dim/40";
  return <span className={`h-2 w-2 shrink-0 rounded-full ${tone}`} />;
}

/** Glyph + colour for a single check's outcome. */
function CheckGlyph({ state }: { state: ChecksState }) {
  if (state === "pending") return <Spinner size={11} />;
  if (state === "failing") return <span className="text-bad">✕</span>;
  return <span className="text-ok">✓</span>; // passing
}

function ExternalLinkIcon() {
  return (
    <svg aria-hidden="true" width="11" height="11" viewBox="0 0 24 24" fill="none" className="shrink-0 opacity-0 group-hover:opacity-60">
      <path d="M14 4h6v6M20 4l-9 9" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M19 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

/**
 * The CI Checks breakdown at the bottom of the diff sidebar. Always shown for an
 * open PR but minimized to just its header until there are actual checks; once
 * checks exist the section is collapsable and lists them line by line — glyph +
 * name — with a row opening the check's run page in the browser. An overall
 * status dot in the header reflects the combined result. Renders nothing when
 * there's no open PR.
 */
export function PrChecksPanel({ view }: { view: WorkstreamView | null }) {
  const id = view?.item.id ?? "";
  const pr = useWorkstreamPr(id, view !== null);
  const [collapsed, setCollapsed] = useState(false);

  const data = pr.data;
  if (!view || !data || data.state !== "open") return null;

  const hasChecks = data.checksList.length > 0;
  const open = hasChecks && !collapsed;

  return (
    <div className="flex max-h-[45%] shrink-0 flex-col border-t border-edge">
      <button
        type="button"
        disabled={!hasChecks}
        onClick={() => setCollapsed((c) => !c)}
        className="flex h-9 shrink-0 items-center gap-2 px-3 text-left disabled:cursor-default"
      >
        <span className="flex w-3 shrink-0 justify-center text-ink-dim">
          {hasChecks ? (
            open ? (
              <ChevronDownIcon size={12} />
            ) : (
              <ChevronRightIcon size={12} />
            )
          ) : null}
        </span>
        <StatusDot state={data.checks} />
        <span className="text-xs font-semibold uppercase tracking-wide text-ink-dim">CI Checks</span>
        <span className="ml-auto text-xs text-ink-dim">
          {hasChecks ? `${data.checksPassed}/${data.checksTotal}` : "none yet"}
        </span>
      </button>
      {open && (
        <div className="min-h-0 flex-1 overflow-y-auto border-t border-edge py-1">
          {data.checksList.map((check, i) => (
            <button
              key={`${check.name}-${i}`}
              type="button"
              disabled={!check.url}
              onClick={() => check.url && void openUrl(check.url)}
              title={check.url ?? check.name}
              className="group flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-edge/40 disabled:cursor-default disabled:hover:bg-transparent"
            >
              <span className="w-3 shrink-0 text-center font-mono">
                <CheckGlyph state={check.state} />
              </span>
              <span className="min-w-0 flex-1 truncate text-ink">{check.name}</span>
              {check.url && <ExternalLinkIcon />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
