import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";

import {
  Button,
  Card,
  ErrorNote,
  Field,
  Spinner,
  TextInput,
} from "@/components/ui";
import {
  useAiStatus,
  useBootstrap,
  useDefaultUsername,
  useInitThoughtsRepo,
  useProbeDefaultThoughts,
} from "@/lib/queries";
import { CliInstallStep } from "@/features/onboarding/CliInstallStep";
import { SetupTerminal } from "@/features/onboarding/SetupTerminal";
import { Step } from "@/features/onboarding/StepShell";
import { RepoSourcePicker, type RepoSource } from "@/features/projects/RepoSourcePicker";
import type { ThoughtsSource } from "@/lib/bindings";
import { useEscape } from "@/lib/keybindings/registry";
import { dismissOnboarding } from "./onboardingState";

const TOTAL_STEPS = 4;


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
          This takes about two minutes. You'll end up with:
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


type AiPhase =
  | { kind: "idle" }
  | { kind: "running"; force: boolean }
  | { kind: "checking" };

function Step2({ onNext }: { onNext: () => void }) {
  const aiStatus = useAiStatus();
  const isInstalled = aiStatus.data?.installed ?? false;
  const agentTool = aiStatus.data?.agentTool ?? "AI tool";

  const [phase, setPhase] = useState<AiPhase>({ kind: "idle" });
  const autoStartedRef = useRef(false);

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
              Complete the prompts below. Arrow keys + Enter to select. This step
              advances automatically when configuration finishes.
            </p>
          </Card>
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


function Step3({ user }: { user: string }) {
  const navigate = useNavigate();
  const bootstrap = useBootstrap();
  const initThoughts = useInitThoughtsRepo();
  const probe = useProbeDefaultThoughts();

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
      seededRef.current = true;
      setSource({ mode: "create", path: "~/thoughts" });
    }
  }, [probe.data, probe.isError]);

  const cfg = bootstrap.data?.config;
  const alreadyConfigured = cfg?.state === "ready" && cfg.configured;

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
            {alreadyConfigured ? "Keep current setup" : "Skip for now"}
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
          </p>
          {alreadyConfigured ? (
            <p className="mt-2 text-sm text-ink">
              ✓ Your thoughts repository is already set up. Keep it, or select a
              different one below and Hyprlayer will switch to it.
            </p>
          ) : (
            foundExisting &&
            source?.mode === "disk" && (
              <p className="mt-2 text-sm text-ink">
                ✓ Using the existing thoughts repo at{" "}
                <span className="break-all font-mono text-xs">{defaultLocation}</span>.
              </p>
            )
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
        {initThoughts.error != null && (
          <ErrorNote error={initThoughts.error} onDismiss={() => initThoughts.reset()} />
        )}
      </div>
    </Step>
  );
}


export function Onboarding() {
  const navigate = useNavigate();
  const [step, setStep] = useState<1 | 2 | 3 | 4>(1);
  const [user, setUser] = useState("");

  const defaultUser = useDefaultUsername();
  const prefilledRef = useRef(false);
  useEffect(() => {
    if (!prefilledRef.current && defaultUser.data) {
      prefilledRef.current = true;
      setUser((cur) => cur || defaultUser.data);
    }
  }, [defaultUser.data]);

  const exit = () => {
    dismissOnboarding();
    void navigate({ to: "/" });
  };
  useEscape(exit);

  return (
    <div className="flex min-h-screen items-start justify-center bg-surface px-4 py-12">
      <div className="w-full max-w-xl">
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
