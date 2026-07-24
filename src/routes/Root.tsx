import {
  Link,
  Outlet,
  useCanGoBack,
  useNavigate,
  useRouter,
  useRouterState,
} from "@tanstack/react-router";
import {
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import { openPath } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";

import {
  useAllWorkstreams,
  useArchiveWorkstream,
  useBootstrap,
  useDetachPr,
  useDoc,
  useDocWebUrl,
  useEditorTargets,
  useReorderProjects,
  useWorkstreamPr,
  useWorktreeFilesCheck,
} from "@/lib/queries";
import { triggerActiveFileSave } from "@/lib/fileSave";
import { docTitle } from "@/features/library/text";
import { WorkstreamActivityDot } from "@/features/session/WorkstreamActivity";
import { startEdgeDrag } from "@/lib/resize";
import { isFolderWorkstream, isGitProject } from "@/lib/domain/projects";
import { GLASS_BAND } from "@/lib/glass";
import { IS_MACOS } from "@/lib/platform";
import { useIsFullscreen } from "@/lib/useFullscreen";
import { useUiStore } from "@/lib/uiStore";
import {
  selectPinnedViews,
  usePinStore,
} from "@/features/workstreams/pinStore";
import { useProjectTreeStore } from "@/features/projects/projectTreeStore";
import { useLibraryScope } from "@/features/library/libraryScope";
import { onboardingDismissed } from "./onboardingState";
import { useCommand, useEscape } from "@/lib/keybindings/registry";
import { useKeybindings } from "@/lib/keybindings/useKeybindings";
import type { Overrides } from "@/lib/keybindings/resolve";
import { effectiveChord } from "@/lib/keybindings/resolve";
import { formatChord } from "@/lib/keybindings/chord";
import { ContextMenu, ContextMenuItem, Spinner } from "@/components/ui";
import { RepoAvatar } from "@/components/RepoAvatar";
import { HyprlayerMark } from "@/components/Logo";
import { WindowControls } from "@/components/WindowControls";
import {
  BookIcon,
  ChevronDownIcon,
  ChevronLeftIcon,
  ChevronRightIcon,
  ClipboardIcon,
  CodeIcon,
  FolderIcon,
  MultiplexerIcon,
  PinIcon,
  PlusIcon,
  ScriptPanelIcon,
  TerminalIcon,
  WarningTriangleIcon,
  ZapIcon,
} from "@/components/icons";
import { useMultiplexer } from "@/features/multiplexer/engines";
import type { EditorTarget } from "@/lib/bindings";
import { SearchPalette } from "@/features/search/SearchPalette";
import { toggleSearch } from "@/features/search/searchStore";
import { FullDiskAccessNotice } from "@/features/system/FullDiskAccessNotice";
import { ConfirmQuitDialog } from "@/features/system/ConfirmQuitDialog";
import { PromptPalette } from "@/features/prompts/PromptPalette";
import { togglePromptPalette } from "@/features/prompts/promptsStore";
import { usePromptHotkeys } from "@/features/prompts/usePromptHotkeys";
import {
  toggleDock as toggleScriptDock,
  toggleDockMaximized,
  useScriptsStore,
} from "@/features/session/scriptsStore";
import { ArchiveWorkstreamDialog } from "@/features/workstreams/ArchiveWorkstreamDialog";
import { WorkstreamGitChip } from "@/features/workstreams/WorkstreamGitChip";
import { useWorkstreamGitActions } from "@/features/workstreams/useGitActions";
import { AheadBehind } from "@/features/workstreams/AheadBehind";
import { RenameWorkstreamDialog } from "@/features/workstreams/RenameWorkstreamDialog";
import { AttachPrDialog } from "@/features/workstreams/AttachPrDialog";
import { AddProjectDialog } from "@/features/projects/AddProjectDialog";
import { ProjectSettingsDialog } from "@/features/projects/ProjectSettingsDialog";
import {
  closeProjectSettings,
  openProjectSettings,
  useProjectSettingsStore,
} from "@/features/projects/projectSettingsStore";
import { RemoveProjectDialog } from "@/features/projects/RemoveProjectDialog";
import {
  useProjectDrag,
  type ProjectDragHandlers,
  type ProjectDragState,
} from "@/features/projects/useProjectDrag";
import { useUpdate, showUpdateToast } from "@/lib/updater";
import { useToast } from "@/lib/toast";
import { Toast } from "@/components/Toast";
import type { Project, WorkstreamView } from "@/lib/bindings";
import { useLastRoute } from "@/lib/lastRoute";

// Stable identity for "no overrides yet" so the dispatcher's memo/effect doesn't
// churn on every Root render before settings have loaded a keybindings section.
const EMPTY_OVERRIDES: Overrides = {};

const PROJECT_SIDEBAR_MIN = 180;
const PROJECT_SIDEBAR_MAX = 360;
// The activity-bar nav rail that sits left of the project panel (fixed width).
const NAV_RAIL_WIDTH = 56;
// The header's left controls (sidebar toggle, dashboard, and — on a project —
// docs) live in an absolutely-positioned cluster, so they're out of flow: the
// header content must be inset past the cluster's right edge or the breadcrumb
// slides under it once the sidebar collapses below the cluster's width. These
// mirror the cluster's Tailwind layout so the inset is derived, not hard-coded
// (and stays correct as controls are added — the old fixed 148 only covered the
// 2-icon macOS cluster and ignored the later-added docs button + all of Linux).
// The cluster's left edge is decided at runtime (see `leftClusterLeft` in the
// component): it clears the macOS traffic lights only when they're actually
// present — i.e. on macOS AND not in fullscreen, which hides them.
const LEFT_CLUSTER_WITH_LIGHTS = 92; // left-[92px]: clears the macOS traffic lights
const LEFT_CLUSTER_NO_LIGHTS = 12; // left-3: no lights to clear (Windows/Linux, or fullscreen)
const LEFT_CLUSTER_ICON = 28; // each control is h-7 w-7
const LEFT_CLUSTER_GAP = 4; // gap-1 between controls
const HEADER_CONTENT_PAD = 16; // pl-4 on the content row, before the breadcrumb
const LEFT_CLUSTER_CLEARANCE = 12; // breathing room between the cluster and the breadcrumb

// Cross-platform vector icons (Lucide family / native-style), currentColor so
// they theme with the bar and stay pixel-crisp at every DPI on macOS + Windows.
// A near-square panel glyph — symmetric padding, rect fills 18/24 of the box —
// rendered at 18 like the editor icon beside it, so both show a ~15px visible
// square and read as an equal pair. The sidebar is marked by a light divider
// line offset to its side (right here), not a filled pane — a quieter glyph
// that still reads as a panel toggle. The button is a fixed h-7 w-7, glyph-only.
function PanelRightIcon() {
  return (
    <svg
      aria-hidden="true"
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
    >
      <rect
        x="3"
        y="3"
        width="18"
        height="18"
        rx="2.5"
        stroke="currentColor"
        strokeWidth="1.8"
      />
      <path d="M15 3v18" stroke="currentColor" strokeWidth="1.8" />
    </svg>
  );
}

/** Mirror of `PanelRightIcon` with the divider line on the left — the header's
 *  explicit toggle for the left project panel (the diff sidebar's right-panel
 *  toggle uses the un-mirrored one). */
function PanelLeftIcon() {
  return (
    <svg
      aria-hidden="true"
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
    >
      <rect
        x="3"
        y="3"
        width="18"
        height="18"
        rx="2.5"
        stroke="currentColor"
        strokeWidth="1.8"
      />
      <path d="M9 3v18" stroke="currentColor" strokeWidth="1.8" />
    </svg>
  );
}

/** Branch-fork glyph for a worktree-backed workstream — it lives on its own
 *  branch in a dedicated worktree. Paired with `MainCheckoutIcon` so a row's
 *  worktree mode reads at a glance. */
function GitBranchIcon() {
  return (
    <svg
      aria-hidden="true"
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      className="shrink-0"
    >
      <circle cx="6" cy="5" r="2.3" stroke="currentColor" strokeWidth="1.8" />
      <circle cx="6" cy="19" r="2.3" stroke="currentColor" strokeWidth="1.8" />
      <circle cx="18" cy="8" r="2.3" stroke="currentColor" strokeWidth="1.8" />
      <path d="M6 7.5v9" stroke="currentColor" strokeWidth="1.8" />
      <path
        d="M18 10.5c0 3-2.5 4.5-5.5 4.5H9.5"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
      />
    </svg>
  );
}

/** Glyph for an in-place workstream: it has no dedicated worktree, so it runs in
 * the project's own checkout. A house reads as "home base / the main checkout"
 * — distinct at a glance from the branch-fork used for worktree-backed rows.
 * Geometry from Lucide's `house` for crisp, properly-weighted rendering. */
function MainCheckoutIcon() {
  return (
    <svg
      aria-hidden="true"
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="shrink-0"
    >
      <path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8" />
      <path d="M3 10a2 2 0 0 1 .709-1.528l7-5.999a2 2 0 0 1 2.582 0l7 5.999A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
    </svg>
  );
}

function ArchiveIcon({ size = 18 }: { size?: number }) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="3" y="4" width="18" height="4" rx="1" />
      <path d="M5 8v10a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8" />
      <path d="M10 12h4" />
    </svg>
  );
}

function SettingsIcon({ size = 18 }: { size?: number }) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
  );
}

/** Rounded-square glyph for the nav rail's Home entry. A simple framed square
 *  reads as "home base / dashboard" and, paired with the accent-filled active
 *  state, becomes the white-on-accent square button from the mock. Tuned to the
 *  mock: a bold, generously rounded outline that fills ~40% of the button. */
function SquareIcon({ size = 18 }: { size?: number }) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
    >
      <rect
        x="2.5"
        y="2.5"
        width="19"
        height="19"
        rx="5.5"
        stroke="currentColor"
        strokeWidth="2.5"
      />
    </svg>
  );
}

/** Isometric cube for the nav rail's Projects entry — a 3D box reads as "a
 *  project / a unit of work," distinct from the flat square (Home) and the book
 *  (Library). Geometry from Lucide's `box`; sized to the mock (~47% of button). */
function CubeIcon({ size = 22 }: { size?: number }) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z" />
      <path d="m3.3 7 8.7 5 8.7-5" />
      <path d="M12 22V12" />
    </svg>
  );
}

/** Book glyph for the nav rail's Library entry — the mock's book: a spine curve
 *  on the left plus the inside-cover line near the foot. Geometry from Lucide's
 *  `book`; distinct from the lined `BookIcon` used for project docs / chat. */
function RailBookIcon({ size = 21 }: { size?: number }) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" />
      <path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2Z" />
    </svg>
  );
}

function decodePathSegment(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function routeProjectPath(pathname: string): string | null {
  const prefix = "/projects/";
  if (!pathname.startsWith(prefix)) return null;
  const segment = pathname.slice(prefix.length).split("/")[0];
  return segment ? decodePathSegment(segment) : null;
}

function routeSection(pathname: string): string {
  if (pathname.startsWith("/projects/")) return "Agent";
  if (pathname.startsWith("/library")) return "Library";
  if (pathname.startsWith("/prompts")) return "Prompt Library";
  if (pathname.startsWith("/settings")) return "Settings";
  if (pathname.startsWith("/archive")) return "Archive";
  if (pathname.startsWith("/onboarding")) return "Setup";
  if (pathname === "/") return "Home";
  return "Workspace";
}

const topIconClass =
  "inline-flex h-7 w-7 items-center justify-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-ink";

/** Primary nav-rail icon button — Home / Projects / Library up top and the
 *  Archive / Settings utilities at the foot. Active wears the redesign's
 *  accent-filled rounded square (white glyph); idle sits dim with a soft
 *  accent-tinted hover. */
// The nav rail's Home button is the persistent accent-filled square from the
// mockup — it always wears the solid fill. Every other entry stays plain until
// its route is active, when it takes the mockup's outlined treatment (accent
// tint + ring + accent glyph). So the active section reads as an outlined box
// while Home stays solid — exactly the two states the mockup shows side by side.
const RAIL_ICON_BASE =
  "flex h-9 w-9 items-center justify-center rounded-[11px] transition-colors";
const railHomeClass = `${RAIL_ICON_BASE} bg-accent text-white shadow-sm`;
const railNavClass = (active: boolean): string =>
  `${RAIL_ICON_BASE} ${
    active
      ? "bg-accent/20 text-accent ring-[1.5px] ring-inset ring-accent/55"
      : "text-ink-dim hover:bg-accent/10 hover:text-ink"
  }`;

/** A row/trigger icon for a detected app: its real rendered icon when the
 *  backend could rasterize one, otherwise a glyph standing in for its kind.
 *  Rasterized macOS app icons sit inside a full-bleed squircle with built-in
 *  padding, so at the bar's nominal 18px the visible glyph reads ~14px — too
 *  light next to the stroke icons. Render the squircle at 22px so the glyph
 *  inside lands at the same optical weight as the bar's stroke icons (e.g.
 *  PanelRightIcon).
 *
 *  These are the apps' real icons, so each carries its own baked-in background
 *  (VS Code's white square, Terminal's black one); they don't recolor with the
 *  theme — there's no dark VS Code icon to swap to. We keep the real icons for
 *  brand recognition rather than dropping to monochrome glyphs. */
function appIcon(app: EditorTarget, size = 22): ReactNode {
  if (app.iconDataUrl) {
    return (
      <img
        src={app.iconDataUrl}
        alt=""
        className="block shrink-0 rounded-[4px]"
        style={{ width: size, height: size }}
      />
    );
  }
  switch (app.kind) {
    case "files":
      return <FolderIcon size={size} />;
    case "terminal":
      return <TerminalIcon size={size} />;
    default:
      return <CodeIcon size={size} />;
  }
}

/** Top-bar dropdown that opens the active worktree (or project) directory in
 *  any installed editor / terminal — detected by the backend so only apps the
 *  user actually has are listed — plus a Copy path fallback. Sits just left of
 *  the changes-sidebar toggle. */
function OpenInMenu({ dir }: { dir: string }) {
  const [open, setOpen] = useState(false);
  const targets = useEditorTargets();

  // The default editor (what the trigger wears) is also pulled to the top of
  // the dropdown so row 1 — and the "1" accelerator — open the user's primary
  // editor, instead of whichever app the backend happened to list first.
  const defaultApp = useMemo(() => {
    const apps = targets.data ?? [];
    return (
      apps.find((a) => a.kind === "editor") ??
      apps.find((a) => a.id !== "files")
    );
  }, [targets.data]);

  const items = useMemo(() => {
    const apps = targets.data ?? [];
    const ordered = defaultApp
      ? [defaultApp, ...apps.filter((a) => a.id !== defaultApp.id)]
      : apps;
    return [
      ...ordered.map((app) => ({
        key: app.id,
        label: app.name,
        icon: appIcon(app),
        run: (): void => {
          // Best-effort: a wrong/uninstalled handler shouldn't surface a scary
          // error for a convenience action, but log so failures are debuggable.
          const p = app.openWith ? openPath(dir, app.openWith) : openPath(dir);
          void p.catch((e) => console.error(`Open in ${app.name} failed`, e));
        },
      })),
      {
        key: "copy-path",
        label: "Copy path",
        icon: <ClipboardIcon size={20} />,
        run: (): void => {
          void navigator.clipboard.writeText(dir);
        },
      },
    ];
  }, [targets.data, defaultApp, dir]);

  // While open, number keys (1–9) trigger the matching row, mirroring the
  // accelerators shown at the right of each item; Escape closes.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        return;
      }
      const n = Number(e.key);
      if (Number.isInteger(n) && n >= 1 && n <= Math.min(items.length, 9)) {
        e.preventDefault();
        setOpen(false);
        items[n - 1].run();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, items]);

  // The trigger wears the default editor's icon (computed above), Conductor-
  // style, so it reads as "open in editor".
  return (
    // `flex items-center` (not a bare block) so the inline-flex button is a
    // flex item — cleanly cross-axis-centered like its sibling icon buttons in
    // the header row. As a plain block wrapper it aligned to the line-box text
    // baseline instead, which pushed the icon a few px high vs. its neighbours.
    <div className="relative flex items-center">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="inline-flex h-7 items-center rounded-md pr-1 text-ink-dim hover:bg-edge/60 hover:text-ink"
        title="Open in editor or terminal"
        aria-label="Open working directory in editor or terminal"
        aria-haspopup="menu"
        aria-expanded={open}
      >
        {/* Icon sits in the same h-7 w-7 square as the right-sidebar button so
            the two bar controls share a footprint; the chevron is a tight
            sidecar that just labels the box as a menu trigger. */}
        <span className="inline-flex h-7 w-7 items-center justify-center">
          {defaultApp ? appIcon(defaultApp, 18) : <CodeIcon size={18} />}
        </span>
        <ChevronDownIcon size={12} className="-ml-1 text-ink-dim/80" />
      </button>
      {open && (
        <>
          <div
            className="fixed inset-0 z-20"
            onClick={() => setOpen(false)}
            aria-hidden="true"
          />
          <div className="absolute right-0 top-full z-30 mt-1.5 min-w-[220px] overflow-hidden rounded-lg border border-edge bg-panel py-1 shadow-xl">
            {items.map((it, i) => (
              <button
                key={it.key}
                type="button"
                onClick={() => {
                  setOpen(false);
                  it.run();
                }}
                className="flex w-full items-center gap-2.5 px-3 py-1.5 text-left text-xs text-ink-dim hover:bg-edge/50 hover:text-ink"
              >
                <span className="shrink-0">{it.icon}</span>
                <span className="flex-1 font-medium">{it.label}</span>
                {i < 9 && (
                  <span className="shrink-0 font-mono text-[10px] text-ink-dim/60">
                    {i + 1}
                  </span>
                )}
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

/** Yellow triangle on a project row when its configured worktree-copy files
 *  don't resolve against the checkout (e.g. a `.env` that doesn't exist on
 *  disk). Hovering lists the offending patterns. Renders nothing when the copy
 *  config is clean or the project has none. The check runs on app start and
 *  refreshes when the copy list changes (see `useWorktreeFilesCheck`). */
/** Per-block transform for the drag-to-reorder gesture: the grabbed block follows
 *  the cursor (no transition); every other block slides by the grabbed block's
 *  height to open the landing gap (CSS transition makes it smooth). */
function projectDragStyle(
  drag: ProjectDragState,
  path: string,
  index: number,
): CSSProperties | undefined {
  if (!drag) return undefined;
  if (drag.path === path) return { transform: `translateY(${drag.dy}px)` };
  const { srcIndex, targetIndex, height } = drag;
  let shift = 0;
  if (srcIndex < targetIndex && index > srcIndex && index <= targetIndex)
    shift = -height;
  else if (srcIndex > targetIndex && index >= targetIndex && index < srcIndex)
    shift = height;
  return {
    transform: shift ? `translateY(${shift}px)` : undefined,
    transition: "transform 180ms ease",
  };
}

function ProjectCopyWarning({ projectPath }: { projectPath: string }) {
  const check = useWorktreeFilesCheck(projectPath);
  const report = check.data;
  const problems = report
    ? [...report.missing, ...report.errors.map((e) => e.entry)]
    : [];
  if (problems.length === 0) return null;
  return (
    <span
      className="flex h-6 w-6 shrink-0 items-center justify-center text-warn"
      title={`Won't be copied to new worktrees:\n${problems.join("\n")}\n\nAdd the file to the repo, or fix the path in Project settings.`}
      aria-label={`Copy file issues: ${problems.join(", ")}`}
    >
      <WarningTriangleIcon size={14} />
    </span>
  );
}

export function Root() {
  const bootstrap = useBootstrap();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const routeHref = useRouterState({ select: (s) => s.location.href });
  const locationSearch = useRouterState({
    select: (s) => s.location.search,
  }) as {
    item?: string;
    doc?: string;
    from?: string;
  };
  // Left project-sidebar collapse lives in the persisted uiStore (alongside the
  // right diff sidebar's `diffHidden`) so the app reopens with the same layout.
  const collapsed = useUiStore((state) => state.projectSidebarCollapsed);
  const setProjectSidebarCollapsed = useUiStore(
    (state) => state.setProjectSidebarCollapsed,
  );
  const toggleProjectSidebar = useUiStore(
    (state) => state.toggleProjectSidebar,
  );
  // Default to the widest we allow; the user can drag it narrower if desired.
  const [sidebarWidth, setSidebarWidth] = useState(PROJECT_SIDEBAR_MAX);
  // True only while the resize handle is being dragged. The panel's width
  // transition (which smooths the collapse/expand) is suppressed during a drag
  // so the panel tracks the cursor instead of trailing it by the animation.
  const [resizingSidebar, setResizingSidebar] = useState(false);

  const projects = useMemo(
    () => bootstrap.data?.projects ?? [],
    [bootstrap.data?.projects],
  );
  const configReady = bootstrap.data?.config.state === "ready";
  const currentProjectPath = routeProjectPath(pathname);
  const currentProject = currentProjectPath
    ? projects.find((project) => project.path === currentProjectPath)
    : null;
  const currentSection = routeSection(pathname);
  // The Library owns the second column itself (its own list panel), so the
  // shell's project tree steps aside on this route.
  const onLibrary = pathname.startsWith("/library");
  // The Prompt library renders its own "Prompts" panel (like the Library's list
  // panel), so the project panel hides there too.
  const onPrompts = pathname.startsWith("/prompts");
  // Breadcrumb title for the selected doc — cache-shared with the detail pane's
  // own `useDoc`, so no extra fetch; disabled (empty id) off the library route.
  const libDoc = useDoc(onLibrary ? (locationSearch.doc ?? "") : "");
  const libDocTitle = libDoc.data ? docTitle(libDoc.data.meta) : undefined;
  // Best-effort "View on GitHub" for the open library doc, surfaced in the
  // window's top-right (cache-shared with the detail pane's own usage).
  const libDocUrl = useDocWebUrl(onLibrary ? (locationSearch.doc ?? "") : "");
  // When the Library was opened from a project (its "Docs" affordance set
  // `from`), resolve that project so the breadcrumb can lead back to it. Dropped
  // if the project no longer exists.
  const backProject =
    onLibrary && locationSearch.from
      ? projects.find((p) => p.path === locationSearch.from)
      : undefined;

  // Remember the project the user is working in so the Library opens scoped to
  // the most relevant project, not every project's docs at once. Keyed on the
  // thoughts-project string (not the recomputed object) so it only writes on a
  // real project switch. Resolved back to a live project before use, so a
  // removed project simply falls back to "all".
  const setLastProject = useLibraryScope((s) => s.setLastProject);
  const lastProject = useLibraryScope((s) => s.lastProject);
  const currentThoughtsProject = currentProject?.thoughtsProject;
  useEffect(() => {
    if (currentThoughtsProject) setLastProject(currentThoughtsProject);
  }, [currentThoughtsProject, setLastProject]);
  const defaultLibraryProject = projects.find(
    (p) => p.thoughtsProject === lastProject,
  )?.thoughtsProject;

  // The sidebar tree: every project's active workstreams, nested beneath it.
  const allWorkstreams = useAllWorkstreams(projects.length > 0);
  const itemsByProject = new Map<string, WorkstreamView[]>();
  for (const view of allWorkstreams.data ?? []) {
    const group = itemsByProject.get(view.item.projectPath) ?? [];
    group.push(view);
    itemsByProject.set(view.item.projectPath, group);
  }

  // ── Drag-to-reorder the project list ──────────────────────────────────────
  // A project block (header + its nested workstreams) drags as a unit: the
  // grabbed block lifts and follows the cursor while the others slide to open its
  // landing gap (see `projectDragStyle` / `useProjectDrag`). Nothing reorders in
  // the DOM until the drop. On drop `dragOrder` holds the new order (rendered via
  // `orderedProjects`) until the persisted bootstrap catches up, so the block
  // never flashes back to its old slot.
  const reorderProjects = useReorderProjects();
  const [dragOrder, setDragOrder] = useState<string[] | null>(null);
  const projectDragRef = useRef<ProjectDragHandlers>({ commit: () => {} });
  const {
    drag: projectDrag,
    suppressClickRef: projectDragSuppress,
    startDrag: startProjectDrag,
  } = useProjectDrag(projectDragRef);

  const orderedProjects = useMemo(() => {
    if (!dragOrder) return projects;
    const byPath = new Map(projects.map((p) => [p.path, p]));
    const out = dragOrder
      .map((path) => byPath.get(path))
      .filter((p): p is Project => p !== undefined);
    // Never drop a project the drag order doesn't know about (e.g. one added
    // out-of-band mid-drag) — append it.
    for (const p of projects) if (!dragOrder.includes(p.path)) out.push(p);
    return out;
  }, [projects, dragOrder]);

  // Repoint the drag handler at the latest project list + mutation each render
  // (the long-lived pointer listeners read it through the ref).
  projectDragRef.current = {
    commit: (from, to) => {
      const paths = projects.map((p) => p.path);
      const next = [...paths];
      const [moved] = next.splice(from, 1);
      next.splice(Math.max(0, Math.min(to, next.length)), 0, moved);
      if (next.every((p, i) => p === paths[i])) return; // unchanged — nothing to persist
      // Show the new order immediately, then persist; clear the override once the
      // write settles (success → the refetch matches; error → the mutation rolls back).
      setDragOrder(next);
      reorderProjects.mutate(next, { onSettled: () => setDragOrder(null) });
    },
  };
  // Pinned worktrees — a user-curated shortcut list rendered above the project
  // tree (see `pinStore`). Resolved against the live active list so a pin whose
  // workstream was archived/deleted simply drops out; `projectByPath` gives each
  // pinned row its project's repo avatar + name (it sits outside its tree).
  const pinnedIds = usePinStore((state) => state.pinnedIds);
  const pinnedSectionCollapsed = usePinStore((state) => state.collapsed);
  const togglePin = usePinStore((state) => state.togglePin);
  // Per-project disclosure in the tree below (VSCode-style folder folding).
  const collapsedProjectPaths = useProjectTreeStore((state) => state.collapsedPaths);
  const toggleProjectCollapsed = useProjectTreeStore((state) => state.toggleCollapsed);
  const togglePinnedSection = usePinStore((state) => state.toggleCollapsed);
  const pinnedViews = selectPinnedViews(pinnedIds, allWorkstreams.data ?? []);
  const projectByPath = new Map(
    projects.map((project) => [project.path, project]),
  );
  const activeWorkstreamId = currentProjectPath
    ? locationSearch.item
    : undefined;
  const activeWorkstream = activeWorkstreamId
    ? (allWorkstreams.data ?? []).find(
        (view) => view.item.id === activeWorkstreamId,
      )
    : undefined;
  // True when we're on a project route with no workstream selected — the board
  // view. Used to suppress the diff-sidebar chrome that only applies to a chat.
  const onBoard = currentSection === "Agent" && !locationSearch.item;
  // Breadcrumb: show "Board" on the project overview, the workstream name when
  // one is selected, or the generic section label everywhere else.
  const crumb =
    currentSection === "Agent"
      ? onBoard
        ? "Board"
        : (activeWorkstream?.item.title ?? "Workstreams")
      : currentSection;

  // A single contextual Back affordance: only on secondary views (library,
  // settings, docs, setup) so users can return to the agent view without
  // re-clicking the worktree in the sidebar. Never shown on the agent view
  // itself, and only when there is actual history to return to.
  const router = useRouter();
  const canGoBack = useCanGoBack();

  // Right-click menu + archive flow for the worktree rows. The view itself
  // carries its project path, so the targets are just the view.
  const navigate = useNavigate();

  // Non-blocking first-run gate: when config is missing and the user hasn't
  // dismissed the wizard this session, navigate to /onboarding. Not a
  // beforeLoad guard — the shell renders immediately and the wizard slides in
  // only once the bootstrap query resolves.
  useEffect(() => {
    if (
      bootstrap.data?.config.state === "missing" &&
      !onboardingDismissed &&
      !pathname.startsWith("/onboarding")
    ) {
      void navigate({ to: "/onboarding" });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bootstrap.data?.config.state, pathname]);

  // Toggling a "global destination" icon (Library / Settings / project docs)
  // while already there backs out to the previous view — or runs `fallback`
  // when this view was the entry point — instead of being a no-op the user has
  // to escape manually. (This replaces the header's dedicated Back button.)
  const closeOrBack = (fallback: () => void) => {
    if (canGoBack) router.history.back();
    else fallback();
  };
  const toggleNav = (
    target: "/library" | "/settings" | "/archive" | "/prompts",
  ) => {
    if (pathname.startsWith(target)) {
      closeOrBack(() => void navigate({ to: "/" }));
    } else if (target === "/library") {
      // Resume the Library where the user left it — scope, filters, and the open
      // document — so returning doesn't force them to re-find it. Read the saved
      // view imperatively (not a subscription) so the shell doesn't re-render on
      // every in-Library interaction. First-ever open (no saved view) falls back
      // to the most-recently-used project scope; "All projects" can still widen.
      const lastSearch = useLibraryScope.getState().lastSearch;
      void navigate({
        to: "/library",
        search:
          lastSearch ??
          (defaultLibraryProject ? { project: defaultLibraryProject } : {}),
      });
    } else {
      void navigate({ to: target });
    }
  };
  // The per-project "Docs" affordance now opens the consolidated Library scoped
  // to this project (the dedicated project-docs route is gone). It leaves the
  // project context — the Library is its own top-level section (rail icon) — so
  // it records `from` (the repo path) to power the header's back-to-project link.
  // It also resumes where the user last left this project's docs — the doc +
  // filters read while scoped to it — re-stamping `from`; a bare scope first time.
  const openProjectDocs = (project: Project) => {
    const saved =
      useLibraryScope.getState().lastSearchByProject[project.thoughtsProject];
    void navigate({
      to: "/library",
      search: {
        ...saved,
        project: project.thoughtsProject,
        from: project.path,
      },
    });
  };

  // Remember the current view so the next launch can restore it (the actual
  // restore happens before first paint in main.tsx, to avoid flashing the
  // dashboard). Skip the onboarding wizard — never a "view" to come back to.
  const rememberRoute = useLastRoute((s) => s.remember);
  useEffect(() => {
    if (!pathname.startsWith("/onboarding")) rememberRoute(routeHref);
  }, [routeHref, pathname, rememberRoute]);

  // When the viewed workstream is archived, fall back to its project view (only
  // when it was the selected item — leaves background archives alone).
  const dismissToProject = useCallback(
    (projectPath: string, id: string) => {
      if (currentProjectPath === projectPath && activeWorkstreamId === id) {
        void navigate({
          to: "/projects/$projectPath",
          params: { projectPath },
          search: {},
        });
      }
    },
    [currentProjectPath, activeWorkstreamId, navigate],
  );

  // Archive-on-merge: when the active workstream's PR is observed merged and
  // the setting is on, archive it automatically. Archive is non-destructive
  // (the worktree moves to archived-contexts/, branch is preserved), so no
  // discard concern. Guarded so it fires once.
  const archiveOnMerge = bootstrap.data?.settings.git?.archiveOnMerge ?? false;
  const autoArchive = useArchiveWorkstream();
  const autoArchivedRef = useRef<Set<string>>(new Set());
  // Observe the active workstream's PR for archive-on-merge. Shares the query
  // cache with the GIT panel's PR controls, so it adds no extra `gh` calls.
  const activePr = useWorkstreamPr(
    activeWorkstreamId ?? "",
    Boolean(activeWorkstream && !isFolderWorkstream(activeWorkstream)),
  );
  useEffect(() => {
    if (!archiveOnMerge || !activeWorkstream) return;
    if (activePr.data?.state !== "merged") return;
    const id = activeWorkstream.item.id;
    if (autoArchivedRef.current.has(id)) return;
    autoArchivedRef.current.add(id);
    const projectPath = activeWorkstream.item.projectPath;
    autoArchive.mutate(
      { id },
      { onSuccess: () => dismissToProject(projectPath, id) },
    );
  }, [
    archiveOnMerge,
    activeWorkstream,
    activePr.data?.state,
    autoArchive,
    dismissToProject,
  ]);

  const [itemMenu, setItemMenu] = useState<{
    x: number;
    y: number;
    view: WorkstreamView;
  } | null>(null);
  const [archiveTarget, setArchiveTarget] = useState<WorkstreamView | null>(
    null,
  );
  const [renameTarget, setRenameTarget] = useState<WorkstreamView | null>(null);
  const [attachPrTarget, setAttachPrTarget] = useState<WorkstreamView | null>(
    null,
  );
  // Detach acts on whichever item's menu is open; the id is passed at mutate time
  // (the menu closes on click, so a hook-arg id would churn before it resolves).
  const detachPr = useDetachPr();
  const [addProjectOpen, setAddProjectOpen] = useState(false);
  const [projectMenu, setProjectMenu] = useState<{
    x: number;
    y: number;
    project: Project;
  } | null>(null);
  const settingsPath = useProjectSettingsStore((s) => s.path);
  // Left-rail scripts/terminal dock: which ws it toggles, whether that dock is
  // open, and whether ANY Run is alive (turns the rail icon green).
  const scriptsActiveWs = useScriptsStore((s) => s.activeWorkstream);
  const scriptsDockOpen = useScriptsStore((s) =>
    s.activeWorkstream ? !!s.dock[s.activeWorkstream]?.open : false,
  );
  const scriptsRunning = useScriptsStore(
    (s) => Object.keys(s.running).length > 0,
  );
  const scriptsPosition = useScriptsStore((s) => s.dockPosition);
  const [removeTarget, setRemoveTarget] = useState<Project | null>(null);
  const diffHidden = useUiStore((state) => state.diffHidden);
  const toggleDiff = useUiStore((state) => state.toggleDiff);
  // Multiplexer mode indicator (set in Settings, not a toggle).
  const mux = useMultiplexer();
  // Toggle the scripts dock's visibility from the rail. When it's docked in the
  // right sidebar, "showing" it also reveals that sidebar (the panel lives in it);
  // "hiding" just closes the dock without collapsing the sidebar.
  const toggleScriptsDock = () => {
    if (!scriptsActiveWs) return;
    const inSidebar = scriptsPosition === "right";
    const visible = scriptsDockOpen && (!inSidebar || !diffHidden);
    if (visible) {
      toggleScriptDock(scriptsActiveWs);
    } else {
      if (!scriptsDockOpen) toggleScriptDock(scriptsActiveWs);
      if (inSidebar && diffHidden) toggleDiff();
    }
  };
  // ⌘⇧F: expand the dock to its full height (or restore it). Per-workstream, so
  // it shares the dock toggle's active-workstream gate.
  const maximizeScriptsDock = () => {
    if (scriptsActiveWs) toggleDockMaximized(scriptsActiveWs);
  };
  const libraryCollapsed = useUiStore((state) => state.libraryCollapsed);
  const toggleLibrary = useUiStore((state) => state.toggleLibrary);

  // The header sidebar toggle acts on whichever left panel the route shows: the
  // Library's list panel on /library, the project tree everywhere else. Both
  // collapse the same way, so one button drives both (and reads each one's
  // shown/hidden state for its label). The Prompt library's panel is fixed
  // (always shown), so the toggle is inert there rather than silently flipping
  // the hidden project tree.
  const sidebarShown = onLibrary
    ? !libraryCollapsed
    : onPrompts
      ? true
      : !collapsed;
  const toggleSidebar = () => {
    if (onPrompts) return;
    if (onLibrary) toggleLibrary();
    else toggleProjectSidebar();
  };

  // Reserve the traffic-light inset only when the lights are actually present:
  // macOS, and not in fullscreen (which hides them). Off macOS there are none,
  // so the cluster hugs the edge. This drives both the cluster's own position
  // and the breadcrumb floor (`minContentOffset`) derived from it.
  const isFullscreen = useIsFullscreen();
  const macTrafficLights = IS_MACOS && !isFullscreen;
  const leftClusterLeft = macTrafficLights
    ? LEFT_CLUSTER_WITH_LIGHTS
    : LEFT_CLUSTER_NO_LIGHTS;
  // Floor for the header inset: clear the absolutely-positioned left cluster so
  // the breadcrumb never collides with it when the sidebar is collapsed. The
  // cluster carries the sidebar toggle (every route has a collapsible left
  // panel) plus the docs button (project routes only) — and, off macOS, the
  // leading Hyprlayer mark (the VSCode-style product icon; macOS shows none, its
  // icon lives in the Dock, so the cluster there clears only the controls).
  const leftClusterIcons =
    (IS_MACOS ? 0 : 1) + 1 + (currentProject && isGitProject(currentProject) ? 1 : 0);
  const leftClusterRight =
    leftClusterLeft +
    leftClusterIcons * LEFT_CLUSTER_ICON +
    (leftClusterIcons - 1) * LEFT_CLUSTER_GAP;
  const minContentOffset =
    leftClusterRight + LEFT_CLUSTER_CLEARANCE - HEADER_CONTENT_PAD;
  // The breadcrumb hugs the left, sitting just past the header's own controls
  // rather than tracking the panels below it. It used to be inset by the rail +
  // project panel so it started where the main content did — which put it over
  // the workstream pane, and made it slide sideways whenever the sidebar was
  // resized or collapsed. `minContentOffset` is the only real constraint: clear
  // the absolutely-positioned left cluster (and, with it, the macOS traffic
  // lights) so the title never collides with the controls.
  const contentOffset = minContentOffset;
  const startSidebarResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (collapsed) return;
    startEdgeDrag(event, {
      startWidth: sidebarWidth,
      min: PROJECT_SIDEBAR_MIN,
      max: PROJECT_SIDEBAR_MAX,
      direction: 1,
      onResize: setSidebarWidth,
      onStart: () => setResizingSidebar(true),
      onEnd: () => setResizingSidebar(false),
    });
  };

  // Commit / Ship gating + actions, shared with the GIT panel's buttons via
  // `useWorkstreamGitActions` so the keyboard commands below stay in lockstep
  // with what the sidebar shows. The buttons (and the PR controls) now live in
  // the GIT panel at the top of the changes sidebar — the header carries no git
  // actions; ⌘ commit/ship remain the path when that sidebar is collapsed.
  const gitActions = useWorkstreamGitActions(activeWorkstream ?? null);

  // Global keyboard shortcuts. The dispatcher reads the user's overrides from
  // bootstrap; each command below registers its handler only while it's usable
  // in the current context (a null handler = "not available now"), so the chord
  // is simply inert elsewhere. Tab/session commands register from ProjectPage.
  useKeybindings(
    bootstrap.data?.settings.keybindings?.overrides ?? EMPTY_OVERRIDES,
  );
  // Effective ⌘K chord for the prompt-library button's tooltip.
  const promptPaletteChord = effectiveChord(
    bootstrap.data?.settings.keybindings?.overrides ?? EMPTY_OVERRIDES,
    "prompts.palette",
    IS_MACOS,
  );
  useCommand("nav.dashboard", () => void navigate({ to: "/" }));
  useCommand("nav.library", () => toggleNav("/library"));
  useCommand("nav.settings", () => toggleNav("/settings"));
  useCommand(
    "nav.projectDocs",
    currentProject && isGitProject(currentProject)
      ? () => openProjectDocs(currentProject)
      : null,
  );
  useCommand("nav.back", canGoBack ? () => router.history.back() : null);
  useCommand("view.toggleProjectSidebar", toggleSidebar);
  // ⌘`: toggle the terminal & scripts dock — the same action as the rail's dock
  // button. Inert without an active workstream (the dock is per-workstream).
  useCommand("terminal.open", scriptsActiveWs ? toggleScriptsDock : null);
  useCommand(
    "terminal.toggleFullscreen",
    scriptsActiveWs ? maximizeScriptsDock : null,
  );
  useCommand(
    "view.toggleChangesSidebar",
    currentSection === "Agent" ? () => toggleDiff() : null,
  );
  useCommand(
    "ws.commit",
    gitActions.canCommit && !gitActions.committing ? gitActions.doCommit : null,
  );
  useCommand(
    "ws.ship",
    gitActions.canShip && !gitActions.shipping ? gitActions.doShip : null,
  );
  useCommand("search.open", () => toggleSearch());
  useCommand("prompts.palette", () => togglePromptPalette());
  // Custom prompt shortcuts (built-ins ride ws.commit / ws.ship above).
  usePromptHotkeys(activeWorkstream ?? null);
  // Escape closes Settings, mirroring how ⌘, toggles it shut. A dialog/menu open
  // over it shadows this on the shared Escape stack, so it closes first.
  useEscape(
    pathname.startsWith("/settings")
      ? () => closeOrBack(() => void navigate({ to: "/" }))
      : null,
  );

  const checkNow = useUpdate((s) => s.checkNow);
  const showToast = useToast((s) => s.show);

  useEffect(() => {
    const unlisten = Promise.all([
      listen("menu:check-for-updates", () => {
        void checkNow().then((result) => {
          if (result === "available") {
            // Single update surface: a bottom-right toast carrying a one-click
            // "Restart to update" button (no separate sidebar pill).
            showUpdateToast();
          } else if (result === "uptodate") {
            const v = useUpdate.getState().currentVersion;
            showToast(
              "You're up to date",
              `You're running the latest version of Hyprlayer${v ? ` (v${v})` : ""}.`,
              { tone: "ok" },
            );
          } else {
            showToast(
              "Couldn't check for updates",
              "Check your connection and try again.",
              {
                tone: "bad",
              },
            );
          }
        });
      }),
      listen("menu:open-settings", () => {
        void navigate({ to: "/settings" });
      }),
      listen("menu:save-file", () => {
        triggerActiveFileSave();
      }),
    ]);
    return () => {
      void unlisten.then((unlisteners) => {
        for (const off of unlisteners) off();
      });
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="flex h-full flex-col bg-surface">
      {/* A tall toolbar with the breadcrumb + window controls vertically
          centered, leaving padding above and below. On macOS the traffic lights
          are pushed down to this bar's center via trafficLightPosition
          (tauri.conf.json y ≈ header height / 2) so they line up with the
          centered content. */}
      {/* Stays opaque: nothing scrolls under it. The window bar sits above the
          h-9 Projects band and the agent tab strip, both of which are solid — so
          a backdrop-blur here would sample flat colour and render nothing. The
          glass lives on those two bands instead, which do have scrollers
          directly beneath them. */}
      <header
        className="relative flex h-11 shrink-0 items-center border-b border-edge bg-panel"
        data-tauri-drag-region
      >
        <div
          className="absolute top-1/2 flex -translate-y-1/2 items-center gap-1"
          style={{ left: leftClusterLeft }}
        >
          {/* Off macOS the native title bar is gone (decorations: false), so the
              top-left corner is ours to fill — lead the cluster with the
              Hyprlayer mark, the VSCode-style product icon. macOS keeps its
              native traffic lights here and shows no in-app logo (its icon lives
              in the Dock), so this renders only off macOS. `drag-region="deep"`
              (not bare) so the glyph itself drags the window, not just the box's
              padding ring — the SVG is the click target, and a bare region only
              drags on a direct hit. */}
          {!IS_MACOS && (
            <span
              className="flex h-7 w-7 shrink-0 items-center justify-center"
              data-tauri-drag-region="deep"
              aria-hidden="true"
            >
              <HyprlayerMark size={20} />
            </span>
          )}
          {/* Explicit sidebar toggle — the discoverable way to collapse or show
              the left panel (the project tree, or the Library's list panel on
              /library). The nav rail's Projects entry also toggles the tree, but
              less obviously. Sits in the traffic-light-aware cluster, so it stays
              clear of the macOS lights and hugs the edge on Windows/fullscreen. */}
          <button
            type="button"
            onClick={toggleSidebar}
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-ink"
            title={sidebarShown ? "Hide sidebar" : "Show sidebar"}
            aria-label={sidebarShown ? "Hide sidebar" : "Show sidebar"}
          >
            <PanelLeftIcon />
          </button>
          {/* Project docs lives here (a fixed left anchor) rather than in the
              right cluster, so it doesn't shift when the diff sidebar inset
              changes. */}
          {currentProject && isGitProject(currentProject) && (
            <button
              type="button"
              onClick={() => openProjectDocs(currentProject)}
              className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-ink"
              title="Project docs"
              aria-label="Project docs"
            >
              <BookIcon size={18} />
            </button>
          )}
        </div>

        {/* marginLeft tracks the sidebar width; not reduced-motion-gated so it
            stays in lockstep with the (also un-gated) sidebar slide on Windows. */}
        {/* No margin transition: the inset no longer follows the sidebar (see
            `contentOffset`), so it only moves when the left cluster itself
            changes — entering fullscreen, or a route that adds the docs button.
            Those are instant elsewhere in the bar, and animating just this row
            would drift it out of step with them. */}
        <div
          className="flex min-w-0 flex-1 items-center justify-between gap-4 pl-4 pr-3"
          style={{ marginLeft: contentOffset }}
          data-tauri-drag-region
        >
          <div
            className="flex min-w-0 items-center gap-2 text-sm"
            data-tauri-drag-region
          >
            {currentProject ? (
              <>
                <Link
                  to="/projects/$projectPath"
                  params={{ projectPath: currentProject.path }}
                  className="truncate font-medium text-ink hover:text-ink"
                >
                  {currentProject.mappedName}
                </Link>
                <span className="text-ink-dim">
                  <ChevronRightIcon />
                </span>
                <span className="truncate text-ink-dim">{crumb}</span>
              </>
            ) : onLibrary ? (
              <>
                {/* Opened from a project ("Docs"): a back link to it, leading the
                    breadcrumb. Goes straight to the project (deterministic — not
                    history, which an in-Library navigation could have muddied). */}
                {backProject && (
                  <>
                    <button
                      type="button"
                      onClick={() =>
                        void navigate({
                          to: "/projects/$projectPath",
                          params: { projectPath: backProject.path },
                          search: {},
                        })
                      }
                      className="flex shrink-0 items-center gap-0.5 font-medium text-ink-dim hover:text-ink"
                      title={`Back to ${backProject.mappedName}`}
                      aria-label={`Back to ${backProject.mappedName}`}
                    >
                      <ChevronLeftIcon size={14} />
                      <span className="max-w-40 truncate">
                        {backProject.mappedName}
                      </span>
                    </button>
                    <span className="text-ink-dim">
                      <ChevronRightIcon />
                    </span>
                  </>
                )}
                {/* "Library" returns to the list (drops the selected doc) while
                    keeping the current scope + filters. */}
                <button
                  type="button"
                  onClick={() =>
                    void navigate({
                      to: "/library",
                      search: (prev) => ({
                        ...prev,
                        doc: undefined,
                        mode: undefined,
                      }),
                    })
                  }
                  className="shrink-0 font-medium text-ink hover:text-ink"
                >
                  Library
                </button>
                {libDocTitle && (
                  <>
                    <span className="text-ink-dim">
                      <ChevronRightIcon />
                    </span>
                    <span className="truncate text-ink-dim">{libDocTitle}</span>
                  </>
                )}
              </>
            ) : (
              <span className="truncate font-medium text-ink">
                {currentSection}
              </span>
            )}
          </div>

          <div className="flex shrink-0 items-center gap-2">
            {/* Multiplexer mode is signalled by the nav-rail Home button taking
                the grid glyph (below) — no separate top-right marker. */}
            {/* "View on GitHub" for the open library doc lives at the window's
                very top-right (onLibrary and currentProject are exclusive). */}
            {onLibrary && libDocUrl.data && (
              <a
                href={libDocUrl.data}
                target="_blank"
                rel="noreferrer"
                className="text-xs font-medium text-ink-dim hover:text-ink"
              >
                View on GitHub ↗
              </a>
            )}
            {currentProject && (
              <>
                {/* Ahead/behind of the active workstream's base — a fallback for the
                    GIT panel header, shown ONLY while the changes sidebar is
                    collapsed (otherwise the panel already shows it, so mirroring it
                    here would just duplicate). Gated on a diverged branch so it
                    stays out of the way when there's nothing to report. */}
                {diffHidden &&
                  activeWorkstream &&
                  (activeWorkstream.git.ahead > 0 ||
                    activeWorkstream.git.behind > 0) && (
                    <AheadBehind
                      ahead={activeWorkstream.git.ahead}
                      behind={activeWorkstream.git.behind}
                      className="px-1"
                    />
                  )}
                {/* New-workstream affordance for the whole project. Preserves the
                    selected workstream (if any) so opening it from a chat doesn't
                    yank you to the board; ProjectPage strips `create` once the
                    dialog has popped. */}
                <button
                  type="button"
                  onClick={() =>
                    void navigate({
                      to: "/projects/$projectPath",
                      params: { projectPath: currentProject.path },
                      search: { item: activeWorkstreamId, create: true },
                    })
                  }
                  className={topIconClass}
                  title="New workstream"
                  aria-label="New workstream"
                >
                  <PlusIcon />
                </button>
                {/* Prompt library: opens the ⌘K palette over the active
                    workstream. Accent bolt among the neutral bar controls;
                    the chord lives in the tooltip. */}
                <button
                  type="button"
                  onClick={() => togglePromptPalette()}
                  className={topIconClass}
                  title={`Prompt library${
                    promptPaletteChord
                      ? ` (${formatChord(promptPaletteChord, IS_MACOS)})`
                      : ""
                  }`}
                  aria-label="Prompt library"
                >
                  <ZapIcon size={16} className="text-accent" />
                </button>
                <OpenInMenu
                  dir={
                    activeWorkstream?.item.worktreePath ?? currentProject.path
                  }
                />
                {currentSection === "Agent" && !onBoard && (
                  <button
                    type="button"
                    onClick={toggleDiff}
                    className={topIconClass}
                    title={
                      diffHidden
                        ? "Show changes sidebar"
                        : "Hide changes sidebar"
                    }
                    aria-label={
                      diffHidden
                        ? "Show changes sidebar"
                        : "Hide changes sidebar"
                    }
                  >
                    <PanelRightIcon />
                  </button>
                )}
              </>
            )}
          </div>
        </div>

        {/* Off-macOS, the native title bar is removed (decorations: false), so
            we draw our own caption buttons flush in the top-right corner — the
            VSCode look. macOS keeps its overlaid traffic lights, so the
            component renders nothing there. */}
        <WindowControls />
      </header>

      <div className="flex min-h-0 flex-1">
        {/* Primary nav rail — the activity-bar column from the redesign. Home,
            Projects, and Library ride up top as accent-filled rounded squares
            when active; Archive + Settings (and the update pill) sit at the
            foot. Always visible, independent of the project panel's collapse. */}
        <nav
          className="flex shrink-0 flex-col items-center border-r border-edge bg-surface py-4"
          style={{ width: NAV_RAIL_WIDTH }}
          aria-label="Primary"
        >
          <div className="flex flex-col items-center gap-3">
            {/* Home — its glyph doubles as the Multiplexer-mode indicator: the
                lone rounded square "splits" into a 2×2 grid of panes while the
                mode is on (set in Settings → Advanced), reverting when off. Still
                navigates home; the mode lives in the tooltip. */}
            <Link
              to="/"
              className={railHomeClass}
              title={
                mux.enabled
                  ? "Home — Multiplexer mode on (Settings → Advanced)"
                  : "Home"
              }
              aria-label={mux.enabled ? "Home (Multiplexer mode on)" : "Home"}
            >
              {mux.enabled ? (
                <MultiplexerIcon size={18} strokeWidth={2} />
              ) : (
                <SquareIcon />
              )}
            </Link>
            <button
              type="button"
              onClick={() => {
                // VS Code-style: clicking the active Projects entry collapses or
                // re-opens the project panel (the nav rail stays). From elsewhere
                // it opens a project with the panel shown. Inert with no projects.
                if (pathname.startsWith("/projects")) {
                  toggleProjectSidebar();
                  return;
                }
                const target = currentProjectPath ?? projects[0]?.path;
                if (target) {
                  setProjectSidebarCollapsed(false);
                  void navigate({
                    to: "/projects/$projectPath",
                    params: { projectPath: target },
                    search: {},
                  });
                }
              }}
              className={railNavClass(pathname.startsWith("/projects"))}
              title={
                pathname.startsWith("/projects")
                  ? collapsed
                    ? "Show projects panel"
                    : "Hide projects panel"
                  : "Projects"
              }
              aria-label="Projects"
            >
              <CubeIcon />
            </button>
            <button
              type="button"
              onClick={() => toggleNav("/library")}
              className={railNavClass(pathname.startsWith("/library"))}
              title="Library"
              aria-label="Library"
            >
              <RailBookIcon />
            </button>
            <button
              type="button"
              onClick={() => toggleNav("/prompts")}
              className={railNavClass(pathname.startsWith("/prompts"))}
              title="Prompt library"
              aria-label="Prompt library"
            >
              {/* Sized with its neighbours (Cube 22 / Book 21), not the 18 of
                  the foot icons — the top group runs visually larger. */}
              <ZapIcon size={21} strokeWidth={1.6} />
            </button>
          </div>
          <div className="mt-auto flex flex-col items-center gap-3">
            <button
              type="button"
              onClick={() => toggleNav("/archive")}
              className={railNavClass(pathname.startsWith("/archive"))}
              title="Archive"
              aria-label="Archive"
            >
              <ArchiveIcon size={18} />
            </button>
            <button
              type="button"
              onClick={toggleScriptsDock}
              className={`${
                scriptsRunning
                  ? `${RAIL_ICON_BASE} relative bg-ship/15 text-ship ring-[1.5px] ring-inset ring-ship/40`
                  : `${railNavClass(scriptsDockOpen)} relative`
              } ${!scriptsActiveWs ? "opacity-50" : ""}`}
              title={
                scriptsRunning
                  ? "Scripts — a Run is active"
                  : "Terminal & scripts dock"
              }
              aria-label="Terminal and scripts dock"
            >
              <ScriptPanelIcon size={18} />
              {scriptsRunning && (
                <span
                  aria-hidden
                  className="absolute right-1 top-1 h-2 w-2 rounded-full bg-ship"
                  style={{
                    boxShadow:
                      "0 0 0 2px var(--color-surface), 0 0 6px var(--color-ship)",
                  }}
                />
              )}
            </button>
            <button
              type="button"
              onClick={() => toggleNav("/settings")}
              className={railNavClass(pathname.startsWith("/settings"))}
              title="Settings"
              aria-label="Settings"
            >
              <SettingsIcon size={18} />
            </button>
          </div>
        </nav>
        {/* Project panel — width-animates between 0 and sidebarWidth so a
            collapse/expand glides instead of snapping. overflow-hidden clips
            the fixed-width inner shell (pinned to the rail edge) so its content
            never reflows mid-animation; the transition is dropped while the
            resize handle is dragged so the panel tracks the cursor. The Library
            route hides it entirely — that route renders its own list panel. The
            width transition is not reduced-motion-gated — Windows maps that to the
            often-off "Animation effects" toggle, which would otherwise snap it. */}
        {!onLibrary && !onPrompts && (
          <aside
            className={`relative shrink-0 overflow-hidden border-r border-edge bg-panel ${
              resizingSidebar ? "" : "transition-[width] duration-200 ease-out"
            }`}
            style={{ width: collapsed ? 0 : sidebarWidth }}
            inert={collapsed}
          >
            <div
              className="absolute inset-y-0 left-0 flex flex-col"
              style={{ width: sidebarWidth }}
            >
              {/* Header band, height-matched to the agent bar (h-9) so the
              "Projects" label + add button sit vertically centered alongside it.
              No bottom rule, VSCode-style: the tree reads as one column, and the
              band's glass already separates it from the list scrolling beneath.
              The agent bar across the seam keeps its border — it divides two
              different panes, where this only titles the one below it. */}
              <div
                data-glass
                className={`relative z-10 flex h-9 shrink-0 items-center justify-between pl-4 pr-3 ${GLASS_BAND}`}
              >
                <p className="text-xs font-medium uppercase tracking-wider text-ink-dim">
                  Projects
                </p>
                <button
                  type="button"
                  onClick={() => setAddProjectOpen(true)}
                  className="flex h-6 w-6 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
                  title="Add project"
                  aria-label="Add project"
                >
                  <PlusIcon size={14} />
                </button>
              </div>
              {/* -mt-9 lifts this scroller up behind the Projects band above, so
                  the list slides under it and the band's backdrop-blur has real
                  content to sample (Tahoe's in-window glass). pt-11 restores the
                  original pt-2 gap on top of the band's h-9, keeping the first
                  row exactly where it was at rest. */}
              <nav className="-mt-9 flex-1 overflow-y-auto px-2 pt-11">
                {bootstrap.isLoading && (
                  <div className="flex justify-center py-4">
                    <Spinner />
                  </div>
                )}
                {!bootstrap.isLoading && projects.length === 0 && (
                  <p className="px-2 py-1 text-sm text-ink-dim">
                    {configReady ? (
                      "No projects mapped yet."
                    ) : (
                      <Link
                        to="/onboarding"
                        className="underline hover:text-ink"
                      >
                        Setup required.
                      </Link>
                    )}
                  </p>
                )}

                {/* Pinned worktrees: a quick-access list above the project tree.
                Hidden when nothing is pinned, so the header never takes space
                until the user pins something. Pin/unpin lives on the worktree
                row's right-click menu. */}
                {pinnedViews.length > 0 && (
                  <div className="mb-1 border-b border-edge/60 pb-1.5">
                    <button
                      type="button"
                      onClick={togglePinnedSection}
                      className="flex w-full items-center gap-1.5 rounded-md px-2 py-1 text-xs font-medium uppercase tracking-wider text-ink-dim hover:text-ink"
                      aria-expanded={!pinnedSectionCollapsed}
                      title={
                        pinnedSectionCollapsed
                          ? "Expand pinned"
                          : "Collapse pinned"
                      }
                    >
                      <ChevronDownIcon
                        size={12}
                        className={`shrink-0 transition-transform ${
                          pinnedSectionCollapsed ? "-rotate-90" : ""
                        }`}
                      />
                      <PinIcon size={12} className="shrink-0" />
                      <span>Pinned</span>
                    </button>
                    {!pinnedSectionCollapsed &&
                      pinnedViews.map((view) => {
                        const project = projectByPath.get(
                          view.item.projectPath,
                        );
                        const projectName = project?.mappedName ?? "";
                        const isActiveItem =
                          currentProjectPath === view.item.projectPath &&
                          activeWorkstreamId === view.item.id;
                        return (
                          <Link
                            key={`pin-${view.item.id}`}
                            to="/projects/$projectPath"
                            params={{ projectPath: view.item.projectPath }}
                            search={{ item: view.item.id }}
                            onContextMenu={(e) => {
                              e.preventDefault();
                              setProjectMenu(null);
                              setItemMenu({ x: e.clientX, y: e.clientY, view });
                            }}
                            className={`flex items-center gap-2 rounded-lg px-2 py-1.5 text-sm hover:bg-edge/50 hover:text-ink ${
                              isActiveItem
                                ? "bg-accent/15 text-ink"
                                : "text-ink-dim"
                            }`}
                            title={`${projectName} · ${view.item.title} · ${view.item.branch}`}
                          >
                            <WorkstreamActivityDot
                              workstreamId={view.item.id}
                            />
                            <RepoAvatar
                              source={project?.originUrl}
                              name={projectName}
                              size={16}
                            />
                            <span className="min-w-0 flex-1 truncate">
                              <span className="text-ink-dim/70">
                                {projectName}
                              </span>
                              <span className="px-1 text-ink-dim/40">›</span>
                              {view.item.title}
                            </span>
                            <WorkstreamGitChip view={view} />
                          </Link>
                        );
                      })}
                  </div>
                )}

                {orderedProjects.map((p, i) => {
                  const children = itemsByProject.get(p.path) ?? [];
                  const isCurrent = currentProjectPath === p.path;
                  const projectHighlight = isCurrent && !activeWorkstreamId;
                  const isProjectCollapsed = collapsedProjectPaths.includes(p.path);
                  const isDragging = projectDrag?.path === p.path;

                  return (
                    <div
                      key={p.path}
                      data-project-path={p.path}
                      style={projectDragStyle(projectDrag, p.path, i)}
                      className={`relative mb-0.5 ${
                        isDragging
                          ? "z-30 rounded-lg bg-panel-2 shadow-2xl ring-1 ring-accent/40"
                          : ""
                      }`}
                    >
                      <div
                        onContextMenu={(e) => {
                          e.preventDefault();
                          setItemMenu(null);
                          setProjectMenu({
                            x: e.clientX,
                            y: e.clientY,
                            project: p,
                          });
                        }}
                        onPointerDown={(e) =>
                          startProjectDrag(e, { path: p.path })
                        }
                        className={`group relative flex select-none items-center overflow-hidden rounded-lg pr-1 ${
                          projectHighlight ? "bg-accent/15" : "hover:bg-edge/50"
                        } ${isDragging ? "cursor-grabbing" : "cursor-grab"}`}
                      >
                        {/* The selected project wears the redesign's accent spine. */}
                        {projectHighlight && (
                          <span
                            className="absolute inset-y-0 left-0 w-[3px] bg-accent"
                            aria-hidden="true"
                          />
                        )}
                        {/* Disclosure, VSCode-style: folds this project's
                            workstreams away. A project with none keeps the same
                            gap so every row's avatar stays on one vertical line.
                            data-no-drag so hitting the chevron toggles instead of
                            starting a project reorder. */}
                        {children.length > 0 ? (
                          <button
                            type="button"
                            data-no-drag
                            onClick={() => toggleProjectCollapsed(p.path)}
                            aria-expanded={!isProjectCollapsed}
                            aria-label={`${isProjectCollapsed ? "Expand" : "Collapse"} ${p.mappedName}`}
                            title={isProjectCollapsed ? "Expand" : "Collapse"}
                            className="ml-1 flex h-5 w-4 shrink-0 items-center justify-center rounded text-ink-dim hover:text-ink"
                          >
                            <ChevronDownIcon
                              size={12}
                              className={`shrink-0 transition-transform ${
                                isProjectCollapsed ? "-rotate-90" : ""
                              }`}
                            />
                          </button>
                        ) : (
                          <span className="ml-1 w-4 shrink-0" aria-hidden="true" />
                        )}
                        <Link
                          to="/projects/$projectPath"
                          params={{ projectPath: p.path }}
                          draggable={false}
                          onClick={(e) => {
                            // A drag that ends on the header shouldn't also navigate.
                            if (projectDragSuppress.current) e.preventDefault();
                          }}
                          className={`flex min-w-0 flex-1 items-center gap-2 py-2 pl-1.5 pr-2.5 text-sm text-ink ${
                            projectHighlight ? "font-semibold" : "font-medium"
                          }`}
                          title={`${p.mappedName} · ${p.backendKind}${p.profile ? ` · ${p.profile}` : ""}`}
                        >
                          <RepoAvatar
                            source={p.originUrl}
                            name={p.mappedName}
                            size={22}
                            className="rounded-md"
                          />
                          <span className="truncate">{p.mappedName}</span>
                          {!p.existsOnDisk && (
                            <span
                              className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn"
                              title="Repository folder is missing on disk"
                            />
                          )}
                        </Link>
                        <span data-no-drag className="contents">
                          <ProjectCopyWarning projectPath={p.path} />
                        </span>
                        {/* Workstream count at rest; the add affordance takes its
                        place on hover (the mockup shows the count). */}
                        {children.length > 0 && (
                          <span className="shrink-0 px-1.5 text-xs tabular-nums text-ink-dim/60 group-hover:hidden">
                            {children.length}
                          </span>
                        )}
                        <Link
                          to="/projects/$projectPath"
                          params={{ projectPath: p.path }}
                          search={{ create: true }}
                          data-no-drag
                          className="hidden h-6 w-6 shrink-0 items-center justify-center rounded text-ink-dim group-hover:flex hover:bg-edge/60 hover:text-ink"
                          title={`New workstream in ${p.mappedName}`}
                          aria-label={`New workstream in ${p.mappedName}`}
                        >
                          <PlusIcon size={14} />
                        </Link>
                      </div>

                      {!isProjectCollapsed &&
                        children.map((view) => {
                          const isActiveItem =
                            isCurrent && activeWorkstreamId === view.item.id;
                        // In-place workstreams run in the project's own checkout (no
                        // dedicated worktree). The row reads as a clean status-dot +
                        // title to match the design, so the distinction lives in the
                        // tooltip rather than a leading glyph.
                        const inplace = view.item.worktreeMode === "inplace";
                        // A folder workstream has no branch at all.
                        const folder = isFolderWorkstream(view);
                        return (
                          <Link
                            key={view.item.id}
                            to="/projects/$projectPath"
                            params={{ projectPath: p.path }}
                            search={{ item: view.item.id }}
                            onContextMenu={(e) => {
                              e.preventDefault();
                              setProjectMenu(null);
                              setItemMenu({ x: e.clientX, y: e.clientY, view });
                            }}
                            // pl-10 puts this title on the project title's x: it
                            // trails the header's chevron (ml-1 + w-4) + avatar.
                            className={`relative flex items-center gap-2.5 overflow-hidden rounded-lg py-2 pl-10 pr-2 text-sm ${
                              isActiveItem
                                ? "bg-accent/15 text-ink"
                                : "text-ink-dim hover:bg-edge/50 hover:text-ink"
                            }`}
                            title={
                              folder
                                ? `${view.item.title} · project folder`
                                : `${view.item.title} · ${view.item.branch}${
                                    inplace ? " · main checkout (no worktree)" : ""
                                  }`
                            }
                          >
                            {isActiveItem && (
                              <span
                                className="absolute inset-y-0 left-0 w-[3px] bg-accent"
                                aria-hidden="true"
                              />
                            )}
                            <WorkstreamActivityDot
                              workstreamId={view.item.id}
                              idle="dot"
                            />
                            <span className="min-w-0 flex-1 truncate">
                              {view.item.title}
                            </span>
                            {/* Worktree-mode indicator: branch-fork for a dedicated
                            worktree, house for an in-place (main-checkout) run.
                            Kept on the trailing edge with the git chip so the
                            leading status dot stays clean (the mock's look). */}
                            <span
                              className="flex shrink-0 items-center text-ink-dim/70"
                              title={
                                folder
                                  ? "Runs in the project folder"
                                  : inplace
                                    ? "Runs in the main checkout (no worktree)"
                                    : "Runs in a dedicated worktree"
                              }
                            >
                              {inplace || folder ? (
                                <MainCheckoutIcon />
                              ) : (
                                <GitBranchIcon />
                              )}
                            </span>
                            <WorkstreamGitChip view={view} />
                          </Link>
                        );
                      })}
                    </div>
                  );
                })}
              </nav>
            </div>
          </aside>
        )}

        {!collapsed && !onLibrary && (
          <div
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize project sidebar"
            className="relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-edge/60"
            onPointerDown={startSidebarResize}
          />
        )}

        <main className="min-w-0 flex-1 overflow-y-auto">
          {/* Boundary for the code-split route chunks (see router.tsx): shows a
              spinner during the one-time fetch of a heavy route (the Library's
              CodeMirror, ProjectPage's xterm), then it's cached. */}
          <Suspense
            fallback={
              <div className="flex h-full items-center justify-center">
                <Spinner size={20} />
              </div>
            }
          >
            <Outlet />
          </Suspense>
        </main>
      </div>

      {itemMenu && (
        <ContextMenu position={itemMenu} onClose={() => setItemMenu(null)}>
          <ContextMenuItem
            onClick={() => {
              void navigate({
                to: "/projects/$projectPath",
                params: { projectPath: itemMenu.view.item.projectPath },
                search: { item: itemMenu.view.item.id },
              });
              setItemMenu(null);
            }}
          >
            Open
          </ContextMenuItem>
          <ContextMenuItem
            onClick={() => {
              togglePin(itemMenu.view.item.id);
              setItemMenu(null);
            }}
          >
            {pinnedIds.includes(itemMenu.view.item.id)
              ? "Unpin workstream"
              : "Pin workstream"}
          </ContextMenuItem>
          <ContextMenuItem
            onClick={() => {
              setRenameTarget(itemMenu.view);
              setItemMenu(null);
            }}
          >
            Rename workstream…
          </ContextMenuItem>
          {/* A folder workstream has no repo to attach a PR to. Detach below
              stays: the escape hatch for a row that got one before this gate. */}
          {!isFolderWorkstream(itemMenu.view) && (
            <ContextMenuItem
              onClick={() => {
                setAttachPrTarget(itemMenu.view);
                setItemMenu(null);
              }}
            >
              {itemMenu.view.item.prNumber === null ? "Attach PR…" : "Change PR…"}
            </ContextMenuItem>
          )}
          {itemMenu.view.item.prNumber !== null && (
            <ContextMenuItem
              onClick={() => {
                detachPr.mutate(itemMenu.view.item.id);
                setItemMenu(null);
              }}
            >
              Detach PR #{itemMenu.view.item.prNumber}
            </ContextMenuItem>
          )}
          <ContextMenuItem
            danger
            onClick={() => {
              setArchiveTarget(itemMenu.view);
              setItemMenu(null);
            }}
          >
            Archive workstream…
          </ContextMenuItem>
        </ContextMenu>
      )}

      {projectMenu && (
        <ContextMenu
          position={projectMenu}
          onClose={() => setProjectMenu(null)}
        >
          <ContextMenuItem
            onClick={() => {
              void navigate({
                to: "/projects/$projectPath",
                params: { projectPath: projectMenu.project.path },
                search: {},
              });
              setProjectMenu(null);
            }}
          >
            Open
          </ContextMenuItem>
          <ContextMenuItem
            onClick={() => {
              void navigate({
                to: "/projects/$projectPath",
                params: { projectPath: projectMenu.project.path },
                search: { create: true },
              });
              setProjectMenu(null);
            }}
          >
            New workstream…
          </ContextMenuItem>
          <ContextMenuItem
            onClick={() => {
              openProjectSettings(projectMenu.project.path);
              setProjectMenu(null);
            }}
          >
            Project settings…
          </ContextMenuItem>
          <ContextMenuItem
            danger
            onClick={() => {
              setRemoveTarget(projectMenu.project);
              setProjectMenu(null);
            }}
          >
            Remove project…
          </ContextMenuItem>
        </ContextMenu>
      )}

      <AddProjectDialog
        open={addProjectOpen}
        onClose={() => setAddProjectOpen(false)}
        onAdded={(project) =>
          void navigate({
            to: "/projects/$projectPath",
            params: { projectPath: project.path },
            search: {},
          })
        }
      />

      {settingsPath && (
        <ProjectSettingsDialog
          projectPath={settingsPath}
          onClose={closeProjectSettings}
        />
      )}

      <RenameWorkstreamDialog
        target={renameTarget}
        onClose={() => setRenameTarget(null)}
      />

      <AttachPrDialog
        target={attachPrTarget}
        onClose={() => setAttachPrTarget(null)}
      />

      <ArchiveWorkstreamDialog
        target={archiveTarget}
        onClose={() => setArchiveTarget(null)}
        onArchived={(id) => {
          // If the archived item was the one selected, drop back to its project.
          if (archiveTarget)
            dismissToProject(archiveTarget.item.projectPath, id);
          setArchiveTarget(null);
        }}
      />

      <RemoveProjectDialog
        target={removeTarget}
        onClose={() => setRemoveTarget(null)}
        onRemoved={(path) => {
          // If the removed project was the one being viewed, drop back home.
          if (currentProjectPath === path) void navigate({ to: "/" });
          setRemoveTarget(null);
        }}
      />

      <SearchPalette />
      <PromptPalette activeWorkstream={activeWorkstream ?? null} />
      <Toast />
      <FullDiskAccessNotice />
      <ConfirmQuitDialog />
    </div>
  );
}
