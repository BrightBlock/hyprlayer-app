import { useMemo, useState } from "react";

import {
  useBootstrap,
  useGithubUsername,
  useRedetectBinaries,
  useSetAppSettings,
} from "../lib/queries";
import type { AppSettings, GitSettings } from "../lib/bindings";
import {
  Badge,
  Button,
  Card,
  CardTitle,
  ErrorNote,
  PageError,
  PageSpinner,
  TextInput,
  Toggle,
} from "../components/ui";
import { BinaryCard } from "../features/system/BinaryCard";
import { UpdatesCard } from "../features/system/updates";
import { THEMES, useThemeStore } from "../lib/theme";

/** [surface, panel-2, accent] preview colors per theme. Kept explicit (rather
 *  than relying on the cascade) since the theme overrides in index.css are
 *  scoped to `html.<name>` and wouldn't resolve on a swatch element. "System"
 *  previews as the default Dark+. */
const SWATCHES: Record<string, [string, string, string]> = {
  system: ["#0c0c0c", "#1c1c1c", "#d97757"],
  dark: ["#0c0c0c", "#1c1c1c", "#d97757"],
  light: ["#faf9f6", "#ebe8e1", "#c25e3d"],
  claude: ["#1f1e1d", "#2a2926", "#d97757"],
  cobalt: ["#193549", "#234a6a", "#66c4ff"],
  matrix: ["#050905", "#161e16", "#3fb950"],
  solarized: ["#002b36", "#0a4756", "#cb4b16"],
};

/** Three-swatch preview of a theme, so the picker reads at a glance. */
function ThemeSwatch({ id }: { id: string }) {
  const [surface, panel, accent] = SWATCHES[id] ?? SWATCHES.dark;
  return (
    <span className="flex shrink-0 overflow-hidden rounded border border-edge">
      <span className="h-5 w-5" style={{ background: surface }} />
      <span className="h-5 w-5" style={{ background: panel }} />
      <span className="h-5 w-5" style={{ background: accent }} />
    </span>
  );
}

function AppearanceSection() {
  const mode = useThemeStore((s) => s.mode);
  const setMode = useThemeStore((s) => s.setMode);

  return (
    <Card>
      <CardTitle>Appearance</CardTitle>
      <p className="mt-1 text-xs text-ink-dim">
        Color theme for the app. “System” follows your OS light/dark setting.
      </p>
      <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3">
        {THEMES.map((theme) => {
          const active = mode === theme.id;
          return (
            <button
              key={theme.id}
              type="button"
              onClick={() => setMode(theme.id)}
              aria-pressed={active}
              className={`flex items-center gap-3 rounded-md border px-3 py-2 text-left text-sm transition-colors ${
                active
                  ? "border-accent bg-panel-2 text-ink"
                  : "border-edge text-ink-dim hover:bg-edge/60 hover:text-ink"
              }`}
            >
              <ThemeSwatch id={theme.id} />
              <span className="truncate">{theme.label}</span>
            </button>
          );
        })}
      </div>
    </Card>
  );
}

/* --------------------------------- Git ----------------------------------- */

const DEFAULT_GIT: Required<GitSettings> = {
  branchPrefix: null,
  deleteBranchOnArchive: true,
  archiveOnMerge: false,
  automerge: false,
};

const DEFAULT_PREFIX = "hyprlayer/";
/** Stand-in slug used to preview how a branch name will read. */
const SAMPLE_SLUG = "add-login";

type PrefixMode = "default" | "github" | "custom";

/** One labelled row with a description and a control on the right. */
function SettingRow({
  title,
  children,
  control,
}: {
  title: string;
  children: React.ReactNode;
  control: React.ReactNode;
}) {
  return (
    <div className="flex items-start justify-between gap-4 py-3.5">
      <div className="min-w-0">
        <p className="text-sm text-ink">{title}</p>
        <p className="mt-0.5 text-xs text-ink-dim">{children}</p>
      </div>
      <div className="shrink-0 pt-0.5">{control}</div>
    </div>
  );
}

function GitSection({ settings }: { settings: AppSettings }) {
  // Fields are individually optional in the bindings (the struct has
  // `#[serde(default)]`), so coalesce each against its built-in default.
  const raw = settings.git ?? {};
  const git: Required<GitSettings> = {
    branchPrefix: raw.branchPrefix ?? DEFAULT_GIT.branchPrefix ?? null,
    deleteBranchOnArchive: raw.deleteBranchOnArchive ?? DEFAULT_GIT.deleteBranchOnArchive,
    archiveOnMerge: raw.archiveOnMerge ?? DEFAULT_GIT.archiveOnMerge,
    automerge: raw.automerge ?? DEFAULT_GIT.automerge,
  };
  const save = useSetAppSettings();
  const username = useGithubUsername();

  // Local mirror of the custom prefix text, so typing stays responsive and we
  // only persist on commit (blur/enter). Seeded from the saved value.
  const initialCustom =
    git.branchPrefix && git.branchPrefix !== `${username.data ?? ""}/`
      ? git.branchPrefix
      : "feature/";
  const [customPrefix, setCustomPrefix] = useState(initialCustom);

  const effectivePrefix = git.branchPrefix ?? DEFAULT_PREFIX;
  const ghPrefix = username.data ? `${username.data}/` : null;

  const mode: PrefixMode =
    git.branchPrefix == null
      ? "default"
      : ghPrefix && git.branchPrefix === ghPrefix
        ? "github"
        : "custom";

  function patch(next: Partial<GitSettings>) {
    save.mutate({ ...settings, git: { ...git, ...next } });
  }

  function pickMode(m: PrefixMode) {
    if (m === "default") patch({ branchPrefix: null });
    else if (m === "github" && ghPrefix) patch({ branchPrefix: ghPrefix });
    else patch({ branchPrefix: customPrefix });
  }

  const radio = (m: PrefixMode, label: React.ReactNode) => (
    <button
      type="button"
      role="radio"
      aria-checked={mode === m}
      onClick={() => pickMode(m)}
      className="flex items-center gap-2 text-left text-sm text-ink"
    >
      <span
        className={`flex h-4 w-4 items-center justify-center rounded-full border ${
          mode === m ? "border-accent" : "border-edge"
        }`}
      >
        {mode === m && <span className="h-2 w-2 rounded-full bg-accent" />}
      </span>
      {label}
    </button>
  );

  return (
    <Card>
      <CardTitle>Git</CardTitle>
      <p className="mt-1 text-xs text-ink-dim">
        Branch naming and the worktree archive &amp; merge behaviour for new workspaces.
      </p>

      {save.isError && (
        <div className="mt-3">
          <ErrorNote error={save.error} />
        </div>
      )}

      <div className="mt-3 divide-y divide-edge/50">
        {/* Branch name prefix */}
        <div className="py-3.5">
          <p className="text-sm text-ink">Branch name prefix</p>
          <p className="mt-0.5 text-xs text-ink-dim">Prefix for new workspace branch names.</p>
          <div className="mt-3 space-y-2.5">
            {radio("default", <span>Default ({DEFAULT_PREFIX})</span>)}
            {radio(
              "github",
              ghPrefix ? (
                <span>GitHub username ({username.data})</span>
              ) : (
                <span className="text-ink-dim">
                  GitHub username{" "}
                  {username.isLoading ? "(detecting…)" : "(unavailable — run gh auth login)"}
                </span>
              ),
            )}
            {radio("custom", <span>Custom</span>)}
            <div className="pl-6">
              <TextInput
                value={customPrefix}
                placeholder="feature/"
                onFocus={() => mode !== "custom" && pickMode("custom")}
                onChange={(e) => setCustomPrefix(e.target.value)}
                onBlur={() => mode === "custom" && patch({ branchPrefix: customPrefix })}
                onKeyDown={(e) => {
                  if (e.key === "Enter") patch({ branchPrefix: customPrefix });
                }}
                className="max-w-xs"
              />
              <p className="mt-1.5 text-xs text-ink-dim">
                Preview:{" "}
                <span className="font-mono text-ink">
                  {effectivePrefix}
                  {SAMPLE_SLUG}
                </span>
              </p>
            </div>
          </div>
        </div>

        <SettingRow
          title="Delete branch on archive"
          control={
            <Toggle
              aria-label="Delete branch on archive"
              checked={git.deleteBranchOnArchive}
              onChange={(v) => patch({ deleteBranchOnArchive: v })}
            />
          }
        >
          Delete the local branch when archiving a workspace. The remote branch is left to
          GitHub’s “automatically delete head branches” setting.
        </SettingRow>

        <SettingRow
          title="Archive on merge"
          control={
            <Toggle
              aria-label="Archive on merge"
              checked={git.archiveOnMerge}
              onChange={(v) => patch({ archiveOnMerge: v })}
            />
          }
        >
          Automatically archive a workspace once its pull request is merged.
        </SettingRow>

        <SettingRow
          title="Automerge"
          control={
            <Toggle
              aria-label="Automerge"
              checked={git.automerge}
              onChange={(v) => patch({ automerge: v })}
            />
          }
        >
          Queue a GitHub auto-merge so a PR lands once required checks pass. Requires auto-merge
          enabled in your GitHub repository settings.
        </SettingRow>
      </div>
    </Card>
  );
}

/* ------------------------------- General --------------------------------- */

function HyprlayerConfigCard({ config }: { config: ReturnType<typeof useBootstrap>["data"] }) {
  const data = config!;
  return (
    <Card>
      <CardTitle>Hyprlayer config</CardTitle>
      {data.config.state === "ready" && (
        <div className="space-y-1 text-sm">
          <p>
            <Badge tone="ok">Found</Badge>
            <span className="selectable ml-2 font-mono text-xs text-ink-dim">
              {data.config.path}
            </span>
          </p>
          <p className="text-ink-dim">
            User <span className="font-mono text-ink">{data.config.user || "—"}</span> · default
            backend <span className="font-mono text-ink">{data.config.backendKind}</span>
            {!data.config.configured && (
              <span className="ml-2">
                <Badge tone="warn">incomplete</Badge>
              </span>
            )}
          </p>
          <p className="mt-2 text-xs text-ink-dim">
            The hyprlayer CLI owns this file; the app reads it and follows changes live.
          </p>
        </div>
      )}
      {data.config.state === "missing" && (
        <div className="space-y-2 text-sm">
          <p>
            <Badge tone="warn">Not set up yet</Badge>
          </p>
          <p className="text-ink-dim">
            No hyprlayer config was found at{" "}
            <span className="selectable font-mono text-xs">{data.config.path}</span>. Add your
            first project from the dashboard — the app sets up your thoughts storage automatically.
          </p>
        </div>
      )}
      {data.config.state === "error" && (
        <ErrorNote error={{ message: data.config.message, remedy: data.config.remedy }} />
      )}
    </Card>
  );
}

function BinariesCard({ data }: { data: NonNullable<ReturnType<typeof useBootstrap>["data"]> }) {
  const redetect = useRedetectBinaries();
  return (
    <Card>
      <div className="flex items-center justify-between">
        <CardTitle className="mb-0">Binaries</CardTitle>
        <Button onClick={() => redetect.mutate()} busy={redetect.isPending}>
          Re-detect
        </Button>
      </div>
      <p className="mt-1 text-xs text-ink-dim">
        The app searches your shell PATH plus common install locations. Set an explicit path if a
        tool isn't found.
      </p>
      <div className="mt-3 space-y-2">
        {data.binaries.map((b) => (
          <BinaryCard key={b.kind} status={b} />
        ))}
      </div>
    </Card>
  );
}

/* ------------------------------ Page shell ------------------------------- */

type SectionId = "general" | "git" | "appearance" | "binaries";

const NAV: Array<{ id: SectionId; label: string }> = [
  { id: "general", label: "General" },
  { id: "git", label: "Git" },
  { id: "appearance", label: "Appearance" },
  { id: "binaries", label: "Binaries" },
];

export function Settings() {
  const bootstrap = useBootstrap();
  const [active, setActive] = useState<SectionId>("general");

  const section = useMemo(() => active, [active]);

  if (bootstrap.isLoading) return <PageSpinner />;
  if (bootstrap.isError) return <PageError error={bootstrap.error} />;

  const data = bootstrap.data!;

  return (
    <div className="mx-auto flex max-w-4xl gap-8 p-8">
      <nav className="w-40 shrink-0">
        <h1 className="mb-3 text-xs font-semibold uppercase tracking-wider text-ink-dim">
          Settings
        </h1>
        <ul className="space-y-0.5">
          {NAV.map((item) => (
            <li key={item.id}>
              <button
                type="button"
                onClick={() => setActive(item.id)}
                aria-current={section === item.id}
                className={`w-full rounded-md px-2.5 py-1.5 text-left text-sm transition-colors ${
                  section === item.id
                    ? "bg-edge text-ink"
                    : "text-ink-dim hover:bg-edge/60 hover:text-ink"
                }`}
              >
                {item.label}
              </button>
            </li>
          ))}
        </ul>
      </nav>

      <div className="min-w-0 flex-1 space-y-4">
        {section === "general" && (
          <>
            <UpdatesCard />
            <HyprlayerConfigCard config={data} />
          </>
        )}
        {section === "git" && <GitSection settings={data.settings} />}
        {section === "appearance" && <AppearanceSection />}
        {section === "binaries" && <BinariesCard data={data} />}
      </div>
    </div>
  );
}
