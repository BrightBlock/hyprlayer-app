import { useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";

import {
  Button,
  Card,
  ErrorNote,
  Field,
  Spinner,
  TextInput,
} from "@/components/ui";
import { Icon } from "@/components/Icon";
import {
  useAiStatus,
  useBootstrap,
  useDefaultUsername,
  useInitThoughtsRepo,
  useProbeDefaultThoughts,
} from "@/lib/queries";
import { CliUpdateNote } from "@/features/settings/AgentBundleSection";
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
        <p className="text-body text-ink-dim">
          This takes about two minutes. You'll end up with:
        </p>
        <ul className="space-y-1 text-body text-ink-dim">
          <li className="flex items-center gap-2">
            <Icon name="check" size="xs" />
            Claude Code and Codex agent workflows linked and ready to use
          </li>
          <li className="flex items-center gap-2">
            <Icon name="check" size="xs" />
            A thoughts repository for your project notes and docs
          </li>
          <li className="flex items-center gap-2">
            <Icon name="check" size="xs" />
            A dashboard where you can add projects and start coding
          </li>
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
          <p className="mt-1 text-ui text-ink-dim">
            Used to attribute your thought documents.
          </p>
        </Field>
      </Card>
    </Step>
  );
}


type AiPhase =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "checking" };

function Step2({ onNext }: { onNext: () => void }) {
  const aiStatus = useAiStatus();
  const isInstalled = aiStatus.data?.installed ?? false;
  const platforms = aiStatus.data?.platforms ?? [];
  const agentPaths = platforms
    .map((platform) => platform.location)
    .filter((location): location is string => !!location);

  const [phase, setPhase] = useState<AiPhase>({ kind: "idle" });
  const startRepair = () => setPhase({ kind: "running" });

  const handleExit = (_code: number | null) => {
    setPhase({ kind: "checking" });
    void aiStatus.refetch().finally(() => setPhase({ kind: "idle" }));
  };

  if (aiStatus.isLoading) {
    return (
      <Step number={3} total={TOTAL_STEPS} title="AI agents">
        <Card className="flex items-center justify-center gap-2 py-12 text-body text-ink-dim">
          <Spinner /> Setting up Claude Code and Codex agents…
        </Card>
      </Step>
    );
  }

  if (phase.kind === "running") {
    return (
      <Step
        number={3}
        total={TOTAL_STEPS}
        title="AI agents"
        footer={
          <Button variant="primary" disabled>
            Next
          </Button>
        }
      >
        <div className="space-y-3">
          <Card>
            <p className="text-body text-ink-dim">
              Repairing the shared Claude Code and Codex agent installation. This step checks the
              result when the command finishes.
            </p>
          </Card>
          <div className="relative h-80 overflow-hidden rounded-lg border border-edge bg-surface">
            <SetupTerminal
              args={["ai", "reinstall"]}
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
      title="AI agents"
      footer={
        <Button variant="primary" onClick={onNext} disabled={!isInstalled}>
          Next
        </Button>
      }
    >
      {phase.kind === "checking" ? (
        <Card className="flex items-center justify-center gap-2 py-12 text-body text-ink-dim">
          <Spinner size="sm" /> Checking status…
        </Card>
      ) : isInstalled ? (
        <Card className="space-y-3">
          <div className="flex items-center gap-2">
            <Icon name="check" size="lg" />
            <p className="text-body font-medium text-ink">
              Claude Code and Codex agents are ready
            </p>
          </div>
          {agentPaths.length > 0 ? (
            <p className="text-ui text-ink-dim">
              {agentPaths.map((location, i) => (
                <span key={location}>
                  {i > 0 && " · "}
                  <span className="font-mono">{location}</span>
                </span>
              ))}
            </p>
          ) : (
            <p className="text-ui text-ink-dim">
              {aiStatus.data?.agentTool ?? "Agent files"} reported ready. Update the Hyprlayer CLI
              to see separate Claude Code and Codex link status.
            </p>
          )}
          <Button type="button" onClick={startRepair} className="text-body">
            Repair installation
          </Button>
        </Card>
      ) : (
        <Card className="space-y-3">
          <p className="text-body text-ink-dim">
            Automatic setup did not finish. Repair the shared installation to link Hyprlayer's
            agents into both Claude Code and Codex.
          </p>
          {aiStatus.error != null && <ErrorNote error={aiStatus.error} />}
          <Button type="button" variant="primary" onClick={startRepair}>
            Repair setup
          </Button>
          {platforms.length === 0 && <CliUpdateNote version={aiStatus.data?.binaryVersion} />}
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
          <p className="text-body text-ink-dim">
            Hyprlayer stores your project notes, plans, and research in a git repository.
          </p>
          {alreadyConfigured ? (
            <p className="mt-2 text-body text-ink">
              <Icon name="check" size="sm" className="mr-1 inline align-[-2px]" />
              Your thoughts repository is already set up. Keep it, or select a
              different one below and Hyprlayer will switch to it.
            </p>
          ) : (
            foundExisting &&
            source?.mode === "disk" && (
              <p className="mt-2 text-body text-ink">
                <Icon name="check" size="sm" className="mr-1 inline align-[-2px]" />
                Using the existing thoughts repo at{" "}
                <span className="break-all text-meta">{defaultLocation}</span>.
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
              className="inline-flex items-center gap-1 text-ui text-ink-dim hover:text-ink"
            >
              <Icon name="chevronLeft" size="sm" /> Back
            </button>
            <button
              type="button"
              onClick={exit}
              className="text-ui text-ink-dim hover:text-ink"
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
