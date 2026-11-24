import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";

import { commands, type DirListing } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { useConnectionStatus } from "@/lib/queries";
import { useUiStore } from "@/lib/uiStore";
import { Badge, Button, Dialog, ErrorNote, Spinner, TextInput, rowClass } from "@/components/ui";
import { Icon } from "@/components/Icon";

type Props = {
  value: string;
  onChange: (next: string) => void;
  placeholder?: string;
  disabled?: boolean;
  autoFocus?: boolean;
  browseTitle: string;
};

/**
 * A path text field plus Browse, browsing the machine the path is used on: the
 * native dialog for a local server, the server-side picker for a remote one.
 */
export function PathField({
  value,
  onChange,
  placeholder,
  disabled = false,
  autoFocus,
  browseTitle,
}: Props) {
  const status = useConnectionStatus();
  const remote = status.data?.host != null;
  const [picking, setPicking] = useState(false);

  const browse = async () => {
    if (remote) {
      setPicking(true);
      return;
    }
    const picked = await open({ directory: true, multiple: false, title: browseTitle });
    if (typeof picked === "string") onChange(picked);
  };

  return (
    <>
      <div className="flex gap-2">
        <TextInput
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          spellCheck={false}
          autoFocus={autoFocus}
          disabled={disabled}
          className="text-meta"
        />
        <Button type="button" onClick={() => void browse()} disabled={disabled}>
          Browse
        </Button>
      </div>
      {picking && (
        <ServerPicker
          title={browseTitle}
          start={value.trim() || null}
          onClose={() => setPicking(false)}
          onPick={(picked) => {
            setPicking(false);
            onChange(picked);
          }}
        />
      )}
    </>
  );
}

/** Eight compact rows; the list scrolls inside this rather than growing the dialog. */
const LIST_HEIGHT = "h-[248px]";
const MAX_ROWS = 200;

/** Highlights the matched run, so why a row survived the filter is visible. */
function Hit({ name, needle }: { name: string; needle: string }) {
  const at = needle ? name.toLowerCase().indexOf(needle.toLowerCase()) : -1;
  if (at < 0) return <>{name}</>;
  return (
    <>
      {name.slice(0, at)}
      <span className="font-medium text-accent">{name.slice(at, at + needle.length)}</span>
      {name.slice(at + needle.length)}
    </>
  );
}

/** Last `/` or `\`, so a Windows server's paths split too. */
function lastSep(value: string): number {
  return Math.max(value.lastIndexOf("/"), value.lastIndexOf("\\"));
}

function sepOf(path: string): string {
  return path.includes("\\") && !path.includes("/") ? "\\" : "/";
}

function ServerPicker({
  title,
  start,
  onClose,
  onPick,
}: {
  title: string;
  start: string | null;
  onClose: () => void;
  onPick: (path: string) => void;
}) {
  const [value, setValue] = useState(start ?? "");
  const [listed, setListed] = useState<{ dir: string | null; listing: DirListing } | null>(null);
  const listing = listed?.listing ?? null;
  const [error, setError] = useState<unknown>(null);
  const showDotFiles = useUiStore((s) => s.showDotFiles);
  const setShowDotFiles = useUiStore((s) => s.setShowDotFiles);

  const cut = lastSep(value);
  // The text after the last separator filters this directory; everything before
  // it names the directory to list. Only the latter is a query — typing a filter
  // must not re-hit the server.
  const dir = value === "" ? null : cut < 0 ? null : value.slice(0, cut) || sepOf(value);
  const segment = cut < 0 ? value : value.slice(cut + 1);

  useEffect(() => {
    let live = true;
    unwrap(commands.browseDirectory(dir))
      .then((next) => {
        if (!live) return;
        setListed({ dir, listing: next });
        setError(null);
        // First open: adopt the server's home so the field shows where we are.
        setValue((current) => (current === "" ? next.path + sepOf(next.path) : current));
      })
      .catch((e) => {
        if (live) setError(e);
      });
    return () => {
      live = false;
    };
  }, [dir]);

  const entries = (listing?.entries ?? []).filter((e) => showDotFiles || !e.name.startsWith("."));
  const matches = segment
    ? entries.filter((e) => e.name.toLowerCase().includes(segment.toLowerCase()))
    : entries;
  const lead = segment
    ? matches.find((e) => e.name.toLowerCase().startsWith(segment.toLowerCase()))
    : undefined;
  // Long paths scroll the input, which would strand a ch-offset overlay.
  const ghost = lead && value.length <= 56 ? lead.name.slice(segment.length) : "";
  const hiddenCount = (listing?.entries.length ?? 0) - entries.length;
  // A directory with thousands of entries should not become thousands of rows;
  // the filter is the way through it, so cap the render and say so.
  const shown = matches.slice(0, MAX_ROWS);
  const overflow = matches.length - shown.length;
  // A typed segment that names a real directory IS the choice — otherwise
  // typing a full path and pressing Choose would hand back its parent.
  const exact = segment ? matches.find((e) => e.isDir && e.name === segment) : undefined;
  const current = !error && (listed?.dir === dir || listing?.path === dir);
  const chosen = !current ? null : exact ? exact.path : (listing?.path ?? null);

  const descend = (entry: { path: string }) => setValue(entry.path + sepOf(entry.path));

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Tab" && lead) {
      e.preventDefault();
      setValue(value + lead.name.slice(segment.length));
    } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      if (chosen) onPick(chosen);
    } else if (e.key === "Enter" && lead?.isDir) {
      e.preventDefault();
      descend(lead);
    }
  };

  return (
    <Dialog
      open
      onClose={onClose}
      title={title}
      size="lg"
      footer={
        <div className="flex items-center justify-between gap-3">
          <span className="min-w-0 truncate text-ui-sm text-ink-dim">
            {matches.length} of {listing?.entries.length ?? 0}
            {hiddenCount > 0 && ` · ${hiddenCount} hidden`}
            {lead && " · Tab completes"}
          </span>
          <div className="flex shrink-0 gap-2">
            <Button type="button" onClick={onClose}>
              Cancel
            </Button>
            <Button
              type="button"
              variant="primary"
              disabled={!chosen}
              onClick={() => chosen && onPick(chosen)}
            >
              Choose this folder
            </Button>
          </div>
        </div>
      }
    >
      <div className="mb-2 flex items-center gap-2">
        {/* Descending is a click, so ascending should be too. `parent` is the
            server's own answer, so no separator is guessed here. */}
        <Button
          type="button"
          className="px-2 py-1 text-ui"
          disabled={!listing?.parent}
          onClick={() => {
            const up = listing?.parent;
            if (up) setValue(up + (up.endsWith(sepOf(up)) ? "" : sepOf(up)));
          }}
          title={listing?.parent ? `Up to ${listing.parent}` : "Already at the top"}
          aria-label="Up one folder"
        >
          Up
        </Button>
        <div className="relative min-w-0 flex-1">
          <TextInput
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder="Type a path, or a name to filter this folder"
            spellCheck={false}
            autoFocus
            className="text-meta"
          />
          {ghost && (
            <span
              aria-hidden="true"
              className="pointer-events-none absolute top-[7px] whitespace-pre bg-accent/35 text-meta text-ink"
              style={{ left: `calc(11px + ${value.length}ch)` }}
            >
              {ghost}
            </span>
          )}
        </div>
        <Button
          type="button"
          className="px-2 py-1 text-ui"
          onClick={() => setShowDotFiles(!showDotFiles)}
          title={showDotFiles ? "Hide dot files" : "Show dot files"}
        >
          {showDotFiles ? "Hide dotfiles" : "Dotfiles"}
        </Button>
      </div>

      <ErrorNote error={error} />

      {!listing && error == null ? (
        <div className="flex items-center gap-2 px-1 py-2 text-ui text-ink-dim">
          <Spinner size="xs" /> Loading…
        </div>
      ) : (
        // Already sorted server-side: directories first, then case-insensitive name.
        <div className={`${LIST_HEIGHT} space-y-0.5 overflow-y-auto`}>
          {matches.length === 0 && (
            <p className="px-1 py-2 text-ui text-ink-dim">
              {segment ? `Nothing here matches “${segment}”.` : "This folder is empty."}
            </p>
          )}
          {shown.map((entry) =>
            entry.isDir ? (
              <button
                key={entry.path}
                type="button"
                onClick={() => descend(entry)}
                className={rowClass({
                  tier: "compact",
                  className: "flex w-full gap-2 px-2 text-left text-meta text-ink",
                })}
              >
                <Icon name="folder" size="xs" className="text-ink-dim" />
                <span className="min-w-0 flex-1 truncate">
                  <Hit name={entry.name} needle={segment} />
                </span>
                {entry.isRepo && <Badge tone="dim">repo</Badge>}
              </button>
            ) : (
              <div
                key={entry.path}
                className={rowClass({
                  tier: "compact",
                  interactive: false,
                  className: "flex gap-2 px-2 text-meta text-ink-dim/70",
                })}
              >
                <Icon name="doc" size="xs" />
                <span className="min-w-0 flex-1 truncate">
                  <Hit name={entry.name} needle={segment} />
                </span>
              </div>
            ),
          )}
          {overflow > 0 && (
            <p className="px-1 py-2 text-ui-sm text-ink-dim">
              {overflow} more — keep typing to narrow.
            </p>
          )}
        </div>
      )}
    </Dialog>
  );
}
