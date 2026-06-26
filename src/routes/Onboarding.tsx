/**
 * /onboarding — first-run setup wizard.
 *
 * Three steps:
 *   1. Welcome + name — required to advance.
 *   2. AI harness — runs `hyprlayer ai configure` in an embedded PTY; skippable
 *      if already installed. Auto-advances on exit 0.
 *   3. Thoughts repo — clone / open / create; "Set up" calls initThoughtsRepo
 *      then lands on the dashboard.
 *
 * Re-enterable from Settings at any time.
 */
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";

import {
  Button,
  Card,
  ErrorNote,
  Field,
  Spinner,
  TextInput,
} from "../components/ui";
import {
  useAiStatus,
  useBootstrap,
  useDefaultUsername,
  useInitThoughtsRepo,
  useProbeDefaultThoughts,
} from "../lib/queries";
import { CliInstallStep } from "../features/onboarding/CliInstallStep";
import { SetupTerminal } from "../features/onboarding/SetupTerminal";
import { Step } from "../features/onboarding/StepShell";
import { RepoSourcePicker, type RepoSource } from "../features/projects/RepoSourcePicker";
import type { ThoughtsSource } from "../lib/bindings";
import { useEscape } from "../lib/keybindings/registry";
import { dismissOnboarding } from "./onboardingState";

/** Total wizard steps — bump when adding/removing a step. */
const TOTAL_STEPS = 4;

// ── Step 1: Welcome + name ────────────────────────────────────────────────────

function Step1({
  user,
  onChange,
  onNext,
}: {
  user: string;
  onChange: (v: string) => void;
  onNext: () => void;
}) {
  return (
    <Step
      number={1}
      total={TOTAL_STEPS}
      title="Welcome to Hyprlayer"
      footer={
        <Button
          variant="primary"
          onClick={onNext}
          disabled={!user.trim()}
        >
          Next
        </Button>
      }
    >
      <Card className="space-y-4">
        <p className="text-sm text-ink-dim">
          Let's get you set up. This takes about two minutes. You'll end up with:
        </p>
        <ul className="space-y-1 text-sm text-ink-dim">
          <li>✦ An AI coding assistant configured and ready to use</li>
          <li>✦ A thoughts repository for your project notes and docs</li>
          <li>✦ A dashboard where you can add projects and start coding</li>
        </ul>
        <Field label="Your name">
          <TextInput
            value={user}
            onChange={(e) => onChange(e.target.value)}
            placeholder="e.g. alice"
            autoFocus
            onKeyDown={(e) => {
              if (e.key === "Enter" && user.trim()) onNext();
            }}
          />
          <p className="mt-1 text-xs text-ink-dim">
            Used to attribute your thought documents.
          </p>
        </Field>
      </Card>
    </Step>
  );
}

// ── Step 2: AI harness ────────────────────────────────────────────────────────

/**
 * A single phase value drives the whole step. Collapsing the booleans into one
 * state is deliberate: `onExit` fires from a Tauri `Channel.onmessage` callback,
 * which React's automatic batching doesn't reliably cover — so multiple
 * `setState`s there can land in separate renders, flashing a stale card between
 * them. One `setPhase` per transition makes every transition atomic.
 */
type AiPhase =
  | { kind: "idle" } // showing the configured / needs-setup card
  | { kind: "running"; force: boolean } // configure terminal is live
  | { kind: "checking" }; // process ended, re-checking status

function Step2({ onNext }: { onNext: () => void }) {
  const aiStatus = useAiStatus();
  const isInstalled = aiStatus.data?.installed ?? false;
  const agentTool = aiStatus.data?.agentTool ?? "AI tool";

  const [phase, setPhase] = useState<AiPhase>({ kind: "idle" });
  const autoStartedRef = useRef(false);

  // Auto-start configuration the first time we learn the user has no AI tool.
  // useLayoutEffect (not useEffect) transitions before paint, so the idle card
  // never flashes on screen before the terminal mounts.
  useLayoutEffect(() => {
    if (
      !aiStatus.isLoading &&
      !isInstalled &&
      phase.kind === "idle" &&
      !autoStartedRef.current
    ) {
      autoStartedRef.current = true;
      setPhase({ kind: "running", force: false });
    }
  }, [aiStatus.isLoading, isInstalled, phase.kind]);

  const start = (force: boolean) => setPhase({ kind: "running", force });

  // The interactive process ended — re-check status, then return to idle (which
  // renders the configured card or a retry prompt). A single `setPhase` per step
  // means no stale card can render in between, even though this runs outside
  // React's event-handler batching.
  const handleExit = (_code: number | null) => {
    setPhase({ kind: "checking" });
    void aiStatus.refetch().finally(() => setPhase({ kind: "idle" }));
  };

  if (aiStatus.isLoading) {
    return (
      <Step number={3} total={TOTAL_STEPS} title="AI harness">
        <Card className="flex items-center justify-center py-12">
          <Spinner />
        </Card>
      </Step>
    );
  }

  // The configure process is live — show the terminal; Next stays disabled.
  if (phase.kind === "running") {
    return (
      <Step
        number={3}
        total={TOTAL_STEPS}
        title="AI harness"
        footer={
          <Button variant="primary" disabled>
            Next
          </Button>
        }
      >
        <div className="space-y-3">
          <Card>
            <p className="text-sm text-ink-dim">
              Complete the prompts below — arrow keys + Enter to select. This step
              advances automatically when configuration finishes.
            </p>
          </Card>
          {/* Fixed height so the layout doesn't reflow */}
          <div className="relative h-80 overflow-hidden rounded-lg border border-edge bg-surface">
            <SetupTerminal
              args={phase.force ? ["ai", "configure", "--force"] : ["ai", "configure"]}
              onExit={handleExit}
              className="h-full"
            />
          </div>
        </div>
      </Step>
    );
  }

  // Idle or checking: configured card, re-check spinner, or a (re)start prompt.
  return (
    <Step
      number={3}
      total={TOTAL_STEPS}
      title="AI harness"
      footer={
        <Button variant="primary" onClick={onNext} disabled={!isInstalled}>
          Next
        </Button>
      }
    >
      {phase.kind === "checking" ? (
        <Card className="flex items-center justify-center gap-2 py-12 text-sm text-ink-dim">
          <Spinner size={14} /> Checking status…
        </Card>
      ) : isInstalled ? (
        <Card className="space-y-3">
          <div className="flex items-center gap-2">
            <span className="text-lg">✓</span>
            <p className="text-sm font-medium text-ink">{agentTool} is configured</p>
          </div>
          <p className="text-sm text-ink-dim">
            Your AI coding assistant is installed and ready. You can proceed to the next step.
          </p>
          <Button type="button" onClick={() => start(true)} className="text-sm">
            Reconfigure
          </Button>
        </Card>
      ) : (
        <Card className="space-y-3">
          <p className="text-sm text-ink-dim">
            Set up your AI coding assistant to continue. It installs the agent and
            its files globally.
          </p>
          <Button type="button" variant="primary" onClick={() => start(false)}>
            Start configuration
          </Button>
        </Card>
      )}
    </Step>
  );
}

// ── Step 3: Thoughts repo ─────────────────────────────────────────────────────

function Step3({ user }: { user: string }) {
  const navigate = useNavigate();
  const bootstrap = useBootstrap();
  const initThoughts = useInitThoughtsRepo();
  const probe = useProbeDefaultThoughts();

  // Source is seeded from the probe once it resolves: if ~/thoughts is already a
  // git repo, default to "open from disk" with the path prefilled; otherwise
  // default to "create" at the same path. `null` until then (we show a spinner).
  const [source, setSource] = useState<RepoSource | null>(null);
  const seededRef = useRef(false);
  useEffect(() => {
    if (seededRef.current) return;
    if (probe.data) {
      seededRef.current = true;
      const path = probe.data.defaultPath;
      setSource(
        probe.data.isGitRepo ? { mode: "disk", path } : { mode: "create", path },
      );
    } else if (probe.isError) {
      // Probe failed — fall back to "create at ~/thoughts" rather than hang.
      seededRef.current = true;
      setSource({ mode: "create", path: "~/thoughts" });
    }
  }, [probe.data, probe.isError]);

  // Already set up? (Re-entering the wizard with a configured config — e.g. via
  // Settings → "Re-run setup".) The backend refuses to rewrite an existing
  // thoughts section, so don't present a setup form that's guaranteed to error —
  // confirm it's done and let the user continue. Changing the repo is a CLI/
  // Settings operation by design.
  const cfg = bootstrap.data?.config;
  const alreadyConfigured = cfg?.state === "ready" && cfg.configured;
  if (alreadyConfigured) {
    return (
      <Step
        number={4}
        total={TOTAL_STEPS}
        title="Your thoughts repository"
        footer={
          <Button variant="primary" onClick={() => void navigate({ to: "/" })}>
            Go to dashboard
          </Button>
        }
      >
        <Card className="space-y-3">
          <div className="flex items-center gap-2">
            <span className="text-lg">✓</span>
            <p className="text-sm font-medium text-ink">
              Your thoughts repository is already set up
            </p>
          </div>
          <p className="text-sm text-ink-dim">
            You're all set. To change your thoughts repository, use Settings or the
            hyprlayer CLI.
          </p>
        </Card>
      </Step>
    );
  }

  const defaultLocation = probe.data?.defaultPath ?? "~/thoughts";
  const foundExisting = probe.data?.isGitRepo === true;

  const canSubmit =
    source != null &&
    ((source.mode === "clone" && Boolean(source.url.trim())) ||
      (source.mode === "disk" && Boolean(source.path.trim())) ||
      (source.mode === "create" && Boolean(source.path.trim())));

  const submit = async () => {
    if (!source || !canSubmit) return;

    let apiSource: ThoughtsSource;
    if (source.mode === "clone") {
      apiSource = {
        mode: "clone",
        url: source.url.trim(),
        // Empty → backend defaults to ~/thoughts; the clone lands AT this path
        // (not under a repo-name subdir), so it's always ~/thoughts.
        location: source.location.trim() || null,
      };
    } else if (source.mode === "disk") {
      apiSource = { mode: "disk", path: source.path.trim() };
    } else {
      apiSource = { mode: "create", path: source.path.trim() };
    }

    await initThoughts.mutateAsync({ user, source: apiSource });
    void navigate({ to: "/" });
  };

  return (
    <Step
      number={4}
      total={TOTAL_STEPS}
      title="Your thoughts repository"
      footer={
        <>
          <Button
            type="button"
            onClick={() => {
              dismissOnboarding();
              void navigate({ to: "/" });
            }}
            disabled={initThoughts.isPending}
          >
            Skip for now
          </Button>
          <Button
            variant="primary"
            onClick={() => void submit()}
            busy={initThoughts.isPending}
            disabled={!canSubmit || initThoughts.isPending}
          >
            {initThoughts.isPending ? "Setting up…" : "Set up"}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <Card>
          <p className="text-sm text-ink-dim">
            Hyprlayer stores your project notes, plans, and research in a git repository.
            Clone an existing one from GitHub, open a folder you already have, or create a fresh one.
          </p>
          {foundExisting && source?.mode === "disk" && (
            <p className="mt-2 text-sm text-ink">
              ✓ Found an existing thoughts repo at{" "}
              <span className="font-mono text-xs">{defaultLocation}</span> — we'll use it.
            </p>
          )}
        </Card>
        {source == null ? (
          <div className="flex justify-center py-6">
            <Spinner />
          </div>
        ) : (
          <RepoSourcePicker
            value={source}
            onChange={setSource}
            modes={["create", "clone", "disk"]}
            labels={{
              create: "Create new",
              clone: "Clone from GitHub",
              disk: "Open existing folder",
            }}
            defaultLocation={defaultLocation}
            disabled={initThoughts.isPending}
          />
        )}
        {initThoughts.error != null && <ErrorNote error={initThoughts.error} />}
      </div>
    </Step>
  );
}

// ── Wizard shell ──────────────────────────────────────────────────────────────

/** The full onboarding wizard. Auto-launched from Root on first run;
 *  also reachable via Settings → "Re-run setup". */
export function Onboarding() {
  const navigate = useNavigate();
  const [step, setStep] = useState<1 | 2 | 3 | 4>(1);
  const [user, setUser] = useState("");

  // Prefill the name from the shell username ($USER) — the same default the CLI
  // uses. Done once when the query resolves, and never clobbers a value the user
  // has already typed (the `cur || …` keeps any edit).
  const defaultUser = useDefaultUsername();
  const prefilledRef = useRef(false);
  useEffect(() => {
    if (!prefilledRef.current && defaultUser.data) {
      prefilledRef.current = true;
      setUser((cur) => cur || defaultUser.data);
    }
  }, [defaultUser.data]);

  // Escape closes the wizard from any step (same as "Exit setup"). Rides the
  // shared Escape stack, so a dialog open over it closes first.
  const exit = () => {
    dismissOnboarding();
    void navigate({ to: "/" });
  };
  useEscape(exit);

  return (
    <div className="flex min-h-screen items-start justify-center bg-surface px-4 py-12">
      <div className="w-full max-w-xl">
        {/* Back/close link — only show on steps 2+ */}
        {step > 1 && (
          <div className="mb-4 flex items-center justify-between">
            <button
              type="button"
              onClick={() => setStep((s) => (s - 1) as 1 | 2 | 3 | 4)}
              className="text-xs text-ink-dim hover:text-ink"
            >
              ← Back
            </button>
            <button
              type="button"
              onClick={exit}
              className="text-xs text-ink-dim hover:text-ink"
            >
              Exit setup
            </button>
          </div>
        )}
        {/* Keyed by step so each change remounts and replays the enter animation. */}
        <div key={step} className="animate-step-in">
          {step === 1 && (
            <Step1
              user={user}
              onChange={setUser}
              onNext={() => setStep(2)}
            />
          )}
          {step === 2 && <CliInstallStep onNext={() => setStep(3)} />}
          {step === 3 && <Step2 onNext={() => setStep(4)} />}
          {step === 4 && <Step3 user={user} />}
        </div>
      </div>
    </div>
  );
}
