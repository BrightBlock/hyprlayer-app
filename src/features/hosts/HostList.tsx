import {
  useEffect,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";

import type { Host, HostsFile, ProvisionStep } from "@/lib/bindings";
import { validateDestination, validateHostList } from "@/lib/domain/hosts";
import { errorMessage } from "@/lib/ipc";
import { useTestHostConnection } from "@/lib/queries";
import { cx, MenuItem, Popover, Spinner, TextInput } from "@/components/ui";
import { CheckIcon, MoreIcon, PlusIcon, XIcon, ZapIcon } from "@/components/icons";
import { retryConnection } from "./reconnect";
import { NO_SERVER, runProvision, STEP_LABEL } from "./serverInstall";

type Props = {
  value: HostsFile;
  onChange: (next: HostsFile) => void;
  /** A switch, not an edit: the caller confirms, writes, and relaunches. */
  onSelect: (next: HostsFile) => void;
  disabled?: boolean;
};

type DraftRow = { label: string; destination: string };

// The status column carries the outcome in two or three words; the ssh text it
// came from is the row's tooltip, since a full failure is a sentence or two and
// would set the width of the whole list. The code is kept because one of them —
// a missing server — is a failure this row can act on rather than only report.
type Probe = { ok: true } | { ok: false; code?: string; short: string; detail: string };

// Codes come from `supervisor::classify_ssh_failure`; anything unrecognised
// still lands here rather than rendering a code at the user.
const FAILURE_SHORT: Record<string, string> = {
  remote_cli_missing: "No server",
  remote_auth_failed: "Auth failed",
  remote_unreachable: "Unreachable",
  remote_platform_unsupported: "Unsupported",
  protocol_version_mismatch: "Version mismatch",
  server_disconnected: "Disconnected",
};

function shortFailure(code: string | undefined): string {
  return (code && FAILURE_SHORT[code]) || "Failed";
}

function toDraft(file: HostsFile): DraftRow[] {
  return (file.hosts ?? []).map((h) => ({ label: h.label, destination: h.destination }));
}

function toRecords(rows: DraftRow[]): Host[] {
  return rows.map((r) => ({ destination: r.destination.trim(), label: r.label.trim() }));
}

const INVALID_FIELD = "border-bad/60 focus:border-bad";
const COMPACT_FIELD = "py-1";

const ROW_BUTTON =
  "shrink-0 rounded px-1 py-0.5 text-xs text-ink-dim hover:bg-edge/60 disabled:cursor-not-allowed disabled:opacity-50";

// The bordered-row anatomy the Prompts sidebar uses (ADR-0006, "Agents: sidebar
// shell"), not rowClass — these rows carry several controls.
// Every control is in flow now, so the row needs no reserved gutter; the header
// shares this inset, and `ROW_BUTTON`, so its `+` lands in the kebab's column.
const ROW_INSET_RIGHT = "pr-1.5";
const ROW_BASE = `flex items-center gap-2 rounded-md border pl-2.5 ${ROW_INSET_RIGHT} py-1 transition-colors`;
const ROW_SELECTED = "border-accent/30 bg-accent/10 text-ink";
const ROW_IDLE = "border-transparent text-ink hover:bg-edge/40";

const BLANK_ADD: DraftRow = { label: "", destination: "" };

// Rows reorder on pointer events, not HTML5 drag-and-drop: the window runs with
// Tauri's `dragDropEnabled` (the composer needs it for file drops,
// SessionChat.tsx:1300), and that native handler swallows dragstart before the
// webview sees it. `index.css` also disables -webkit-user-drag app-wide.
const DRAG_THRESHOLD_PX = 4;

export function HostList({ value, onChange, onSelect, disabled }: Props) {
  const [draft, setDraft] = useState<DraftRow[]>(() => toDraft(value));
  const lastSent = useRef(JSON.stringify(value));
  const [editing, setEditing] = useState<Set<number>>(new Set());
  const [probes, setProbes] = useState<Record<string, Probe>>({});
  const [testing, setTesting] = useState<string | null>(null);
  // One at a time, like `testing`: this pushes 16MB over ssh, and a second
  // concurrent push is a way to saturate a link, not a feature.
  const [installing, setInstalling] = useState<{ destination: string; step: ProvisionStep } | null>(
    null,
  );
  const [dragging, setDragging] = useState<number | null>(null);
  const [dragOver, setDragOver] = useState<number | null>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const drag = useRef<{ from: number; startY: number; moved: boolean } | null>(null);
  // A reorder ends with a click on the row it started from; without this the
  // drop would also select that host.
  const suppressClick = useRef(false);
  const [menuFor, setMenuFor] = useState<number | null>(null);
  const menuAnchor = useRef<HTMLElement | null>(null);
  const test = useTestHostConnection();

  useEffect(() => {
    const next = JSON.stringify(value);
    if (next === lastSent.current) return;
    lastSent.current = next;
    setDraft(toDraft(value));
    setEditing(new Set());
  }, [value]);

  const active = value.active ?? null;
  const rowErrors = validateHostList(draft);

  function buildFile(rows: DraftRow[], nextActive: string | null): HostsFile | null {
    if (validateHostList(rows).size > 0) return null;
    const hosts = toRecords(rows);
    return {
      hosts,
      active: hosts.some((h) => h.destination === nextActive) ? nextActive : null,
    };
  }

  function commit(rows: DraftRow[], nextActive: string | null = active) {
    const file = buildFile(rows, nextActive);
    if (!file) return;
    lastSent.current = JSON.stringify(file);
    onChange(file);
  }

  function edit(i: number, patch: Partial<DraftRow>) {
    setDraft((rows) => rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  }

  function remove(i: number) {
    const next = draft.filter((_, j) => j !== i);
    setDraft(next);
    setEditing(new Set());
    commit(next);
  }

  function move(from: number, to: number) {
    if (from === to || to < 0 || to >= draft.length) return;
    const next = [...draft];
    next.splice(to, 0, ...next.splice(from, 1));
    setDraft(next);
    setEditing(new Set());
    commit(next);
  }

  /** Which row the pointer is over, clamped to the ends so a drag past the
      list still lands somewhere. */
  function rowIndexAt(clientY: number): number | null {
    const items = Array.from(listRef.current?.querySelectorAll<HTMLElement>("li[data-row]") ?? []);
    if (!items.length) return null;
    const hit = items.findIndex((li) => {
      const r = li.getBoundingClientRect();
      return clientY >= r.top && clientY <= r.bottom;
    });
    if (hit !== -1) return hit;
    return clientY < items[0].getBoundingClientRect().top ? 0 : items.length - 1;
  }

  function onRowPointerDown(e: ReactPointerEvent<HTMLDivElement>, i: number) {
    if (disabled || editing.has(i) || e.button !== 0) return;
    // A press on a control is a click, never the start of a reorder.
    if ((e.target as HTMLElement).closest("[data-control], input")) return;
    // Deliberately no setPointerCapture here: capture retargets the gesture to
    // this div, so the click would land on the row instead of the select button
    // inside it and a plain click would never select. Capture starts below,
    // once the press is actually a drag.
    drag.current = { from: i, startY: e.clientY, moved: false };
  }

  function onRowPointerMove(e: ReactPointerEvent<HTMLDivElement>) {
    const state = drag.current;
    if (!state) return;
    if (!state.moved) {
      if (Math.abs(e.clientY - state.startY) < DRAG_THRESHOLD_PX) return;
      state.moved = true;
      // Now it is a drag, so take the pointer: this keeps the move and the drop
      // coming to this row even when the cursor runs past the list.
      e.currentTarget.setPointerCapture(e.pointerId);
      setDragging(state.from);
    }
    setDragOver(rowIndexAt(e.clientY));
  }

  function onRowPointerUp(e: ReactPointerEvent<HTMLDivElement>) {
    const state = drag.current;
    drag.current = null;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    const to = rowIndexAt(e.clientY);
    if (state?.moved && to !== null) {
      suppressClick.current = true;
      move(state.from, to);
    }
    setDragging(null);
    setDragOver(null);
  }

  function doneEditing(i: number, rows: DraftRow[]) {
    if (validateHostList(rows).has(i)) return;
    setEditing((prev) => {
      const next = new Set(prev);
      next.delete(i);
      return next;
    });
  }

  /** The way out of a row that will not commit: put back what was last saved
      and leave edit mode, so a blanked field cannot trap the list. */
  function cancelEditing(i: number) {
    const saved = toDraft(value)[i];
    if (saved) setDraft((rows) => rows.map((r, j) => (j === i ? saved : r)));
    setEditing((prev) => {
      const next = new Set(prev);
      next.delete(i);
      return next;
    });
  }

  const rowCommitProps = (i: number) => ({
    onBlur: (e: FocusEvent<HTMLInputElement>) => {
      commit(draft);
      if (e.currentTarget.closest("li")?.contains(e.relatedTarget as Node | null)) return;
      doneEditing(i, draft);
    },
    onKeyDown: (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // Escape belongs to the row here, not to the dialog behind it.
        e.stopPropagation();
        cancelEditing(i);
        return;
      }
      if (e.key !== "Enter") return;
      commit(draft);
      doneEditing(i, draft);
    },
  });

  async function runTest(row: DraftRow) {
    const host = { destination: row.destination.trim(), label: row.label.trim() };
    setTesting(host.destination);
    const record = (probe: Probe) => setProbes((prev) => ({ ...prev, [host.destination]: probe }));
    try {
      const probe = await test.mutateAsync(host);
      if (probe.ok) {
        record({ ok: true });
        // The connection surface above is still reporting the failure this host
        // just disproved. Reconnecting is what makes the two agree — and it is
        // the same work a relaunch would do, without the relaunch.
        if (host.destination === active) void retryConnection();
      } else {
        record({
          ok: false,
          code: probe.error?.code,
          short: shortFailure(probe.error?.code),
          detail: errorMessage(probe.error ?? "Could not connect."),
        });
      }
    } catch (e) {
      record({ ok: false, short: "Failed", detail: errorMessage(e) });
    } finally {
      setTesting(null);
    }
  }

  // Provisioning a host it has not switched to yet is the point: the probe above
  // already knows the server is missing, and learning that only after a restart
  // into a blocked window cost a second one to get out of.
  async function runInstall(row: DraftRow) {
    const host = { destination: row.destination.trim(), label: row.label.trim() };
    setInstalling({ destination: host.destination, step: "probe" });
    try {
      await runProvision(host, (step) =>
        setInstalling((cur) => (cur?.destination === host.destination ? { ...cur, step } : cur)),
      );
      setInstalling(null);
      // The row's own probe is what says it worked; nothing here claims it.
      await runTest(row);
    } catch (e) {
      setInstalling(null);
      setProbes((prev) => ({
        ...prev,
        // Still the code that offers the action: a failed push leaves the server
        // as missing as it was, so retrying stays one click rather than two.
        [host.destination]: {
          ok: false,
          code: NO_SERVER,
          short: "Install failed",
          detail: errorMessage(e),
        },
      }));
    }
  }

  const [adding, setAdding] = useState<DraftRow | null>(null);
  const addDestination = adding?.destination.trim() ?? "";
  const addRule = validateDestination(addDestination);
  const addDuplicate =
    addDestination && draft.some((r) => r.destination.trim() === addDestination)
      ? "Another host already uses this destination."
      : null;
  const addError = addRule ? `Destination ${addRule}.` : addDuplicate;
  const canAdd = !!adding && !!adding.label.trim() && !!addDestination && !addError;

  function confirmAdd() {
    if (!adding || !canAdd) return;
    const next = [...draft, adding];
    setDraft(next);
    commit(next);
    setAdding(null);
  }

  function onAddKeyDown(e: KeyboardEvent) {
    if (e.key === "Enter") confirmAdd();
    else if (e.key === "Escape") setAdding(null);
  }

  const showEmptyState = draft.length === 0 && !adding;

  return (
    <div>
      {/* Same right inset, border box *and* control geometry as a row, so `+`
          lands in the kebab's column. A shared right edge alone does not do it:
          a wider button centres its icon further left, which is the 5px the
          32px `IconButton` used to sit off by. */}
      <div
        className={`flex items-center justify-between gap-3 border border-transparent ${ROW_INSET_RIGHT}`}
      >
        <h3 className="text-2xs font-semibold uppercase tracking-wider text-ink-dim">Hosts</h3>
        <button
          type="button"
          className={ROW_BUTTON}
          disabled={disabled || !!adding}
          aria-label="Add host"
          title="Add host"
          onClick={() => setAdding(BLANK_ADD)}
        >
          <PlusIcon size={14} />
        </button>
      </div>
      {showEmptyState && (
        <p className="mt-1 text-xs text-ink-dim">No hosts yet.</p>
      )}
      {(draft.length > 0 || adding) && (
        <ul ref={listRef} className="mt-1.5 space-y-0.5">
          {draft.map((row, i) => {
            const name = row.label.trim() || "host";
            const destination = row.destination.trim();
            const rowError = rowErrors.get(i);
            const isEditing = editing.has(i);
            const isActive = !!destination && destination === active;
            const probe = probes[destination];
            const installStep =
              installing?.destination === destination ? installing.step : null;
            // Only a probe can raise this, so an untested host offers nothing —
            // the ⚡ is still the way in.
            const canInstall = !probe?.ok && probe?.code === NO_SERVER;
            // Probe results only. The live connection is not repeated here: the
            // dialog header already carries it, and this row is already marked
            // as the active one by its check and tint — so a "Connected" chip
            // said the same thing a third time, on the one row that could not
            // have meant anything else.
            const status = probe?.ok
              ? { dot: "bg-ok", tone: "text-ink-dim", label: "Answered", detail: null }
              : probe
                ? { dot: "bg-bad", tone: "text-bad", label: probe.short, detail: probe.detail }
                : null;
            return (
              <li key={i} data-row>
                <div
                  className={cx(
                    ROW_BASE,
                    isActive ? ROW_SELECTED : ROW_IDLE,
                    !isEditing && !disabled && "cursor-grab select-none active:cursor-grabbing",
                    dragging === i && "opacity-50",
                    dragging !== null && dragOver === i && dragging !== i && "ring-1 ring-accent/40",
                  )}
                  onPointerDown={(e) => onRowPointerDown(e, i)}
                  onPointerMove={onRowPointerMove}
                  onPointerUp={onRowPointerUp}
                  onPointerCancel={() => {
                    drag.current = null;
                    setDragging(null);
                    setDragOver(null);
                  }}
                >
                  {!isEditing ? (
                    <>
                      {/* The row itself selects the server — a real button so
                          the keyboard gets it too, sibling to the icons rather
                          than wrapping them. */}
                      <button
                        type="button"
                        className="flex min-w-0 flex-1 items-center gap-2 text-left"
                        disabled={disabled || !destination || isActive || installing !== null}
                        aria-current={isActive}
                        title={isActive ? `${name} is the active server` : `Use ${name}`}
                        onClick={() => {
                          if (suppressClick.current) {
                            suppressClick.current = false;
                            return;
                          }
                          const next = buildFile(draft, destination);
                          if (next) onSelect(next);
                        }}
                      >
                        <span className="w-4 shrink-0 text-accent">
                          {isActive && <CheckIcon size={13} />}
                        </span>
                        <span className="shrink-0 truncate text-sm text-ink">{row.label}</span>
                        <span className="min-w-0 flex-1 truncate font-mono text-sm text-ink-dim">
                          {row.destination}
                        </span>
                        {installStep ? (
                          <span className="flex shrink-0 items-center gap-1.5">
                            <Spinner size={11} />
                            <span className="text-xs text-ink-dim">
                              {STEP_LABEL[installStep]}…
                            </span>
                          </span>
                        ) : (
                          status && (
                            <span
                              className="flex shrink-0 items-center gap-1.5"
                              title={status.detail ?? undefined}
                            >
                              <span className={cx("h-1.5 w-1.5 rounded-full", status.dot)} />
                              <span className={cx("text-xs", status.tone)}>{status.label}</span>
                            </span>
                          )
                        )}
                      </button>
                      {canInstall && !installStep && (
                        <button
                          data-control
                          type="button"
                          className={`${ROW_BUTTON} text-accent`}
                          disabled={disabled || installing !== null}
                          onClick={() => void runInstall(row)}
                          title={`Install the server on ${name}`}
                        >
                          Install
                        </button>
                      )}
                      <button
                        data-control
                        type="button"
                        className={ROW_BUTTON}
                        disabled={disabled || !destination || testing === destination || installing !== null}
                        onClick={() => void runTest(row)}
                        title={`Test ${name}`}
                        aria-label={`Test ${name}`}
                      >
                        {testing === destination ? <Spinner size={13} /> : <ZapIcon size={13} />}
                      </button>
                      <button
                        data-control
                        type="button"
                        className={ROW_BUTTON}
                        disabled={disabled || installStep !== null}
                        title={`More actions for ${name}`}
                        aria-label={`More actions for ${name}`}
                        onClick={(e) => {
                          menuAnchor.current = e.currentTarget;
                          setMenuFor(i);
                        }}
                      >
                        <MoreIcon size={14} />
                      </button>
                    </>
                  ) : (
                    <>
                      <TextInput
                        aria-label="Label"
                        placeholder="Nexus"
                        className={cx("w-36 shrink-0", COMPACT_FIELD, rowError?.label && INVALID_FIELD)}
                        value={row.label}
                        disabled={disabled}
                        onChange={(e) => edit(i, { label: e.target.value })}
                        {...rowCommitProps(i)}
                      />
                      <div className="min-w-0 flex-1">
                        <TextInput
                          aria-label="Destination"
                          placeholder="jt@nexus.local"
                          className={cx("font-mono", COMPACT_FIELD, rowError?.destination && INVALID_FIELD)}
                          value={row.destination}
                          disabled={disabled}
                          onChange={(e) => edit(i, { destination: e.target.value })}
                          {...rowCommitProps(i)}
                        />
                      </div>
                    </>
                  )}
                  {isEditing && (
                    <>
                      <button
                        type="button"
                        data-control
                        className={ROW_BUTTON}
                        onClick={() => {
                          commit(draft);
                          doneEditing(i, draft);
                        }}
                        disabled={disabled || !!rowError}
                        title="Done"
                        aria-label={`Done editing ${name}`}
                      >
                        <CheckIcon size={13} />
                      </button>
                      <button
                        type="button"
                        data-control
                        className={ROW_BUTTON}
                        // Keep focus in the field: blurring it would commit the
                        // very edit this button exists to throw away.
                        onMouseDown={(e) => e.preventDefault()}
                        onClick={() => cancelEditing(i)}
                        disabled={disabled}
                        title="Cancel"
                        aria-label={`Cancel editing ${name}`}
                      >
                        <XIcon size={13} />
                      </button>
                    </>
                  )}
                </div>
                {isEditing && (rowError?.label || rowError?.destination) && (
                  <p role="alert" className="mt-1 pl-1 text-xs text-bad">
                    {rowError.label ?? rowError.destination}
                  </p>
                )}
              </li>
            );
          })}
          {adding && (
            <li className="rounded-md border border-dashed border-edge p-1.5">
              <div className="flex items-center gap-2">
                <TextInput
                  aria-label="Label"
                  placeholder="Nexus"
                  className={cx("w-36 shrink-0", COMPACT_FIELD)}
                  value={adding.label}
                  disabled={disabled}
                  autoFocus
                  onChange={(e) => setAdding({ ...adding, label: e.target.value })}
                  onKeyDown={onAddKeyDown}
                />
                <div className="min-w-0 flex-1">
                  <TextInput
                    aria-label="Destination"
                    placeholder="jt@nexus.local"
                    className={cx("font-mono", COMPACT_FIELD, addError && INVALID_FIELD)}
                    value={adding.destination}
                    disabled={disabled}
                    onChange={(e) => setAdding({ ...adding, destination: e.target.value })}
                    onKeyDown={onAddKeyDown}
                  />
                </div>
                <button
                  type="button"
                  className={`${ROW_BUTTON} hover:text-bad`}
                  onClick={() => setAdding(null)}
                  disabled={disabled}
                  aria-label="Cancel add"
                >
                  <XIcon size={12} />
                </button>
                <button
                  type="button"
                  className={`${ROW_BUTTON} text-accent`}
                  onClick={confirmAdd}
                  disabled={disabled || !canAdd}
                  aria-label="Add"
                >
                  <PlusIcon size={12} />
                </button>
              </div>
              {addError && (
                <p role="alert" className="mt-1 pl-1 text-xs text-bad">
                  {addError}
                </p>
              )}
            </li>
          )}
        </ul>
      )}
      {/* One popover for the list, anchored to whichever kebab opened it. */}
      <Popover
        open={menuFor !== null}
        onClose={() => setMenuFor(null)}
        anchorRef={menuAnchor}
        align="end"
        className="min-w-36"
      >
        <MenuItem
          onClick={() => {
            if (menuFor !== null) setEditing((prev) => new Set(prev).add(menuFor));
            setMenuFor(null);
          }}
        >
          Edit
        </MenuItem>
        <MenuItem
          danger
          onClick={() => {
            if (menuFor !== null) remove(menuFor);
            setMenuFor(null);
          }}
        >
          Remove
        </MenuItem>
      </Popover>
    </div>
  );
}
