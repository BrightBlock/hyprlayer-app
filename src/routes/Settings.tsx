import { useBootstrap, useRedetectBinaries } from "../lib/queries";
import { Badge, Button, Card, CardTitle, ErrorNote, PageError, PageSpinner } from "../components/ui";
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

function AppearanceCard() {
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

export function Settings() {
  const bootstrap = useBootstrap();
  const redetect = useRedetectBinaries();

  if (bootstrap.isLoading) return <PageSpinner />;
  if (bootstrap.isError) return <PageError error={bootstrap.error} />;

  const data = bootstrap.data!;

  return (
    <div className="mx-auto max-w-2xl space-y-4 p-8">
      <h1 className="text-xl font-semibold">Settings</h1>

      <UpdatesCard />

      <AppearanceCard />

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
    </div>
  );
}
