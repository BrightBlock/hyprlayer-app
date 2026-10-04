import { useEffect, useRef, useState, type ReactNode } from "react";

import type { Project } from "@/lib/bindings";
import {
  SIDE_PANEL_MIN,
  useUiStore,
  WORKSHOP_LIST_MAX,
  WORKSHOP_LIST_MIN,
} from "@/lib/uiStore";
import { useViewport } from "@/lib/useViewport";
import { Icon, type IconName } from "./Icon";
import { RepoAvatar } from "./RepoAvatar";
import { Badge, cx, PaneSeparator, Popover } from "./ui";

export const WORKSHOP_NAV_WIDTH = 220;

export type WorkshopNavSection = {
  key: string;
  label: string;
  icon: IconName;
  count: number | null;
  soon?: boolean;
};

export type WorkshopChrome = {
  sections: WorkshopNavSection[];
  active: string;
  onSection: (key: string) => void;
  scope: {
    projects: Project[];
    value: string | null;
    onChange: (path: string | null) => void;
  } | null;
  footer: ReactNode;
};

const BORDERED_ROW = "flex w-full items-center gap-2 rounded-md border text-left transition-colors";
const ROW_SELECTED = "border-accent/30 bg-accent/10 text-ink";
const ROW_IDLE = "border-transparent text-ink hover:bg-edge/40";

export function WorkshopFrame({
  nav,
  list,
  detail,
  navKey,
}: {
  nav: ReactNode;
  list: ReactNode;
  detail: ReactNode;
  navKey: string;
}) {
  const { narrow } = useViewport();
  const navCollapsed = useUiStore((s) => s.workshopNavCollapsed);
  const narrowDrawer = useUiStore((s) => s.narrowDrawer);
  const setNarrowDrawer = useUiStore((s) => s.setNarrowDrawer);
  const listWidth = useUiStore((s) => s.workshopListWidth);
  const setListWidth = useUiStore((s) => s.setWorkshopListWidth);
  const [dragWidth, setDragWidth] = useState<number | null>(null);

  useEffect(() => {
    if (useUiStore.getState().narrowDrawer === "workshop") setNarrowDrawer(null);
  }, [navKey, setNarrowDrawer]);

  const navShown = narrow ? narrowDrawer === "workshop" : !navCollapsed;
  const width = narrow ? SIDE_PANEL_MIN : (dragWidth ?? listWidth);

  return (
    <div className="relative flex h-full min-h-0">
      {navShown && (
        <aside
          className={cx(
            "flex shrink-0 flex-col border-r border-edge bg-panel",
            narrow && "absolute inset-y-0 left-0 z-30 shadow-lg",
          )}
          style={{ width: WORKSHOP_NAV_WIDTH }}
        >
          {nav}
        </aside>
      )}
      <div className="flex min-h-0 shrink-0 flex-col border-r border-edge" style={{ width }}>
        {list}
      </div>
      {!narrow && (
        <PaneSeparator
          label="Resize workshop list"
          width={listWidth}
          min={WORKSHOP_LIST_MIN}
          max={WORKSHOP_LIST_MAX}
          onResize={setDragWidth}
          onEnd={() =>
            setDragWidth((live) => {
              if (live !== null) setListWidth(live);
              return null;
            })
          }
        />
      )}
      <section className="min-w-0 flex-1 overflow-y-auto">{detail}</section>
    </div>
  );
}

export function WorkshopNav({
  chrome,
  action,
  filters,
}: {
  chrome: WorkshopChrome;
  action?: ReactNode;
  filters?: ReactNode;
}) {
  return (
    <>
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-3">
        {(action || chrome.scope) && (
          <div className="space-y-2">
            {action}
            {chrome.scope && (
              <WorkshopScopePicker
                projects={chrome.scope.projects}
                value={chrome.scope.value}
                onChange={chrome.scope.onChange}
              />
            )}
          </div>
        )}
        <nav className="space-y-0.5" aria-label="Workshop">
          {chrome.sections.map((s) => (
            <NavSectionRow
              key={s.key}
              icon={s.icon}
              label={s.label}
              count={s.count}
              soon={s.soon}
              selected={s.key === chrome.active}
              onClick={() => chrome.onSection(s.key)}
            />
          ))}
        </nav>
        {filters}
      </div>
      <div className="mt-auto shrink-0 border-t border-edge px-4 py-3">{chrome.footer}</div>
    </>
  );
}

export function NavSectionRow({
  icon,
  label,
  count,
  soon,
  selected,
  onClick,
}: {
  icon: IconName;
  label: string;
  count: number | null;
  soon?: boolean;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={selected}
      className={cx(BORDERED_ROW, "px-2.5 py-2 text-body", selected ? ROW_SELECTED : ROW_IDLE)}
    >
      <span className={cx("shrink-0", selected ? "text-accent" : "text-ink-dim")}>
        <Icon name={icon} />
      </span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {soon ? (
        <Badge tone="dim">Soon</Badge>
      ) : (
        count !== null && <span className="shrink-0 text-ui text-ink-dim opacity-70">{count}</span>
      )}
    </button>
  );
}

export function NavFilterRow({
  label,
  count,
  dot,
  selected,
  onClick,
  onContextMenu,
}: {
  label: string;
  count?: number;
  dot?: string;
  selected: boolean;
  onClick: () => void;
  onContextMenu?: (at: { x: number; y: number }) => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      onContextMenu={
        onContextMenu
          ? (e) => {
              e.preventDefault();
              onContextMenu({ x: e.clientX, y: e.clientY });
            }
          : undefined
      }
      aria-current={selected}
      className={cx(BORDERED_ROW, "px-2.5 py-1.5 text-body", selected ? ROW_SELECTED : ROW_IDLE)}
    >
      {dot && (
        <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: dot }} />
      )}
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {count !== undefined && (
        <span className="shrink-0 text-ui text-ink-dim opacity-70">{count}</span>
      )}
    </button>
  );
}

export function NavGroupLabel({ children, trailing }: { children: ReactNode; trailing?: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-2 px-1.5 pb-1">
      <p className="min-w-0 truncate text-label text-ink-dim">{children}</p>
      {trailing}
    </div>
  );
}

export function WorkshopScopePicker({
  projects,
  value,
  onChange,
}: {
  projects: Project[];
  value: string | null;
  onChange: (path: string | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const current = projects.find((p) => p.path === value) ?? null;

  const choose = (path: string | null) => {
    onChange(path);
    setOpen(false);
  };

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="Workshop scope"
        className="flex w-full items-center gap-2 rounded-md border border-edge bg-panel-2 px-2.5 py-1.5 text-body hover:bg-edge/40"
      >
        {current ? (
          <RepoAvatar source={current.originUrl} name={current.mappedName} size={14} className="rounded" />
        ) : (
          <Icon name="layers" size="sm" className="text-ink-dim" />
        )}
        <span className="min-w-0 flex-1 truncate text-left font-medium text-ink">
          {current?.mappedName ?? "Global · all projects"}
        </span>
        <Icon name="chevronDown" size="xs" className="text-ink-dim" />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={triggerRef}
        matchAnchorWidth
        className="max-h-72 overflow-y-auto"
      >
        <ScopeItem active={!value} onClick={() => choose(null)}>
          <Icon name="layers" size="sm" />
          <span className="min-w-0 flex-1 truncate">Global · all projects</span>
        </ScopeItem>
        {projects.map((p) => (
          <ScopeItem key={p.path} active={p.path === value} onClick={() => choose(p.path)}>
            <RepoAvatar source={p.originUrl} name={p.mappedName} size={14} className="rounded" />
            <span className="min-w-0 flex-1 truncate">{p.mappedName}</span>
          </ScopeItem>
        ))}
      </Popover>
    </>
  );
}

function ScopeItem({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      aria-current={active}
      className={`flex w-full items-center gap-2 rounded px-2.5 py-1.5 text-left text-body ${
        active ? "bg-accent/10 text-ink" : "text-ink-dim hover:bg-panel-2 hover:text-ink"
      }`}
    >
      {children}
    </button>
  );
}

export function ListCardRow({
  title,
  badges,
  description,
  meta,
  leading,
  selected,
  onClick,
  onContextMenu,
}: {
  title: ReactNode;
  badges?: ReactNode;
  description?: string;
  meta?: ReactNode;
  leading?: ReactNode;
  selected: boolean;
  onClick: () => void;
  onContextMenu?: (at: { x: number; y: number }) => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      onContextMenu={
        onContextMenu
          ? (e) => {
              e.preventDefault();
              onContextMenu({ x: e.clientX, y: e.clientY });
            }
          : undefined
      }
      aria-current={selected}
      className={cx(
        "flex w-full items-start gap-2 rounded-md border px-2.5 py-2 text-left transition-colors",
        selected ? ROW_SELECTED : ROW_IDLE,
      )}
    >
      {leading && <span className="flex h-5 shrink-0 items-center">{leading}</span>}
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span className="min-w-0 truncate text-body font-medium text-ink">{title}</span>
          {badges && <span className="flex shrink-0 items-center gap-1">{badges}</span>}
        </span>
        {description && (
          <span className="mt-0.5 line-clamp-1 text-ui text-ink-dim">{description}</span>
        )}
        {meta && <span className="mt-0.5 block truncate text-meta text-ink-dim">{meta}</span>}
      </span>
    </button>
  );
}

export function ListHeader({ children, actions }: { children: ReactNode; actions?: ReactNode }) {
  return (
    <div className="flex h-9 shrink-0 items-center gap-2 border-b border-edge pl-4 pr-3">
      <p className="min-w-0 flex-1 truncate text-ui text-ink-dim">{children}</p>
      {actions && <div className="flex shrink-0 items-center gap-0.5">{actions}</div>}
    </div>
  );
}
