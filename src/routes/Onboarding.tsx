import { useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";

import { BLANK_CTA, BLANK_CTA_SECONDARY } from "@/components/BlankState";
import { BrandMark, harnessBrand } from "@/components/BrandMark";
import {
  Button,
  Card,
  ErrorNote,
  Field,
  Spinner,
  TextInput,
} from "@/components/ui";
import { Icon } from "@/components/Icon";
import { PathField } from "@/components/PathField";
import {
  invalidateAgentBundle,
  useAgentHarnessCounts,
  useAiStatus,
  useBootstrap,
  useDefaultUsername,
  useInitThoughtsRepo,
  useProbeDefaultThoughts,
} from "@/lib/queries";
import { CliUpdateNote } from "@/features/settings/AgentBundleSection";
import { CliInstallStep } from "@/features/onboarding/CliInstallStep";
import { SetupTerminal } from "@/features/onboarding/SetupTerminal";
import { CARD_CLASS, CheckDot, Step } from "@/features/onboarding/StepShell";
import { WizardFrame } from "@/features/onboarding/WizardFrame";
import { RepoSourcePicker, type RepoSource } from "@/features/projects/RepoSourcePicker";
import type { ThoughtsSource } from "@/lib/bindings";
import { plural } from "@/lib/format";
import { useEscape } from "@/lib/keybindings/registry";
import { IS_MACOS } from "@/lib/platform";
import { dismissOnboarding } from "./onboardingState";

const AGENTS_TITLE = "Link your agents";
const AGENTS_LEDE =
  "Hyprlayer's agents, skills and commands are installed once and shared by Claude Code and Codex.";

function NameStep({
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
      title="Your name"
      onNext={onNext}
      nextEnabled={!!user.trim()}
    >
      <Card className={CARD_CLASS}>
        <Field label="Name">
          <TextInput
            value={user}
            onChange={(e) => onChange(e.target.value)}
            placeholder="e.g. alice"
            autoFocus
            className="w-80"
            onKeyDown={(e) => {
              if (e.key === "Enter" && user.trim()) onNext();
            }}
          />
        </Field>
      </Card>
    </Step>
  );
}


type AiPhase =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "checking" };

function AgentsStep({ onBack, onNext }: { onBack: () => void; onNext: () => void }) {
  const aiStatus = useAiStatus();
  const counts = useAgentHarnessCounts();
  const isInstalled = aiStatus.data?.installed ?? false;
  const platforms = aiStatus.data?.platforms ?? [];

  const [phase, setPhase] = useState<AiPhase>({ kind: "idle" });
  const startRepair = () => setPhase({ kind: "running" });

  const handleExit = (_code: number | null) => {
    setPhase({ kind: "checking" });
    invalidateAgentBundle();
    void aiStatus.refetch().finally(() => setPhase({ kind: "idle" }));
  };

  if (aiStatus.isLoading) {
    return (
      <Step title={AGENTS_TITLE} lede={AGENTS_LEDE} onBack={onBack}>
        <Card className={`${CARD_CLASS} flex items-center justify-center gap-2 py-12 text-body text-ink-dim`}>
          <Spinner /> Setting up Claude Code and Codex agents…
        </Card>
      </Step>
    );
  }

  if (phase.kind === "running") {
    return (
      <Step
        title={AGENTS_TITLE}
        lede={AGENTS_LEDE}
        onBack={onBack}
        onNext={onNext}
        nextEnabled={false}
      >
        <div className="space-y-3">
          <Card className={CARD_CLASS}>
            <p className="text-body text-ink-dim">
              Repairing the shared Claude Code and Codex agent installation.
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
      title={AGENTS_TITLE}
      lede={AGENTS_LEDE}
      onBack={onBack}
      onNext={onNext}
      nextEnabled={isInstalled}
    >
      {phase.kind === "checking" ? (
        <Card className={`${CARD_CLASS} flex items-center justify-center gap-2 py-12 text-body text-ink-dim`}>
          <Spinner size="sm" /> Checking status…
        </Card>
      ) : isInstalled ? (
        <Card className={CARD_CLASS}>
          <div className="flex items-center gap-3 pb-3.5">
            <CheckDot className="bg-ok" />
            <p className="text-body font-semibold text-ink">Claude Code and Codex agents are ready</p>
          </div>
          {platforms.length > 0 ? (
            platforms.map((p) => {
              const count = counts.data?.find((c) => c.platformId === p.id);
              return (
                <div key={p.id ?? p.name} className="flex items-center gap-3 border-t border-edge-2 py-2.5">
                  <BrandMark brand={harnessBrand(p.id)} size="sm" />
                  <span className="w-[104px] text-body font-semibold text-ink">{p.name}</span>
                  {p.location && (
                    <span className="selectable flex-1 font-mono text-meta text-ink-dim">
                      {`${p.location.replace(/\/+$/, "")}/agents`}
                    </span>
                  )}
                  {count && (
                    <span className="text-meta text-ink-dim">
                      {plural(count.agents, "agent")} · {plural(count.skills, "skill")}
                    </span>
                  )}
                  {p.installed ? (
                    <Icon name="check" size="sm" className="text-ok" />
                  ) : (
                    <Icon name="warning" size="sm" className="text-warn-ink" />
                  )}
                </div>
              );
            })
          ) : (
            <p className="border-t border-edge-2 py-2.5 text-ui text-ink-dim">
              {aiStatus.data?.agentTool ?? "Agent files"} reported ready. Update the Hyprlayer CLI
              to see separate Claude Code and Codex link status.
            </p>
          )}
          <div className="border-t border-edge-2 pt-3.5">
            <Button type="button" onClick={startRepair}>
              <Icon name="tool" size="sm" /> Repair installation
            </Button>
          </div>
        </Card>
      ) : (
        <Card className={`${CARD_CLASS} space-y-3`}>
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


function ThoughtsStep({ user, onBack }: { user: string; onBack: () => void }) {
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
      title="Thoughts repository"
      lede="Hyprlayer stores your project notes, plans, and research in a git repository."
      onBack={onBack}
      footer={
        <>
          <Button
            type="button"
            className={BLANK_CTA_SECONDARY}
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
            className={BLANK_CTA}
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
        {alreadyConfigured ? (
          <p className="text-ui text-ink">
            <Icon name="check" size="sm" className="mr-1 inline align-[-2px]" />
            Your thoughts repository is already set up. Keep it, or select a different one below
            and Hyprlayer will switch to it.
          </p>
        ) : (
          foundExisting &&
          source?.mode === "disk" && (
            <p className="text-ui text-ink">
              <Icon name="check" size="sm" className="mr-1 inline align-[-2px]" />
              Using the existing thoughts repo at{" "}
              <span className="break-all text-meta">{defaultLocation}</span>.
            </p>
          )
        )}
        {source == null ? (
          <div className="flex justify-center py-6">
            <Spinner />
          </div>
        ) : (
          <RepoSourcePicker
            value={source}
            onChange={setSource}
            modes={["create", "clone", "disk"]}
            variant="cards"
            labels={{
              create: "Create new",
              clone: "Clone from GitHub",
              disk: "Open existing folder",
            }}
            icons={{
              create: <Icon name="circlePlus" size="sm" />,
              clone: <BrandMark brand="github" size="sm" />,
              disk: <Icon name="folder" size="sm" />,
            }}
            descriptions={{
              create: "Start an empty git repo for your notes",
              clone: "Pull an existing thoughts repo",
              disk: `Use a repo already on this ${IS_MACOS ? "Mac" : "computer"}`,
            }}
            defaultLocation={defaultLocation}
            disabled={initThoughts.isPending}
            createSlot={
              source.mode === "create" && (
                <Card className={CARD_CLASS}>
                  <Field label="Location">
                    <PathField
                      value={source.path}
                      onChange={(path) => setSource({ mode: "create", path })}
                      placeholder="~/thoughts"
                      autoFocus
                      disabled={initThoughts.isPending}
                      browseTitle="Choose where to create the repo"
                    />
                  </Field>
                </Card>
              )
            }
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
    <WizardFrame step={step} onExit={exit}>
      {step === 1 && <NameStep user={user} onChange={setUser} onNext={() => setStep(2)} />}
      {step === 2 && <CliInstallStep onBack={() => setStep(1)} onNext={() => setStep(3)} />}
      {step === 3 && <AgentsStep onBack={() => setStep(2)} onNext={() => setStep(4)} />}
      {step === 4 && <ThoughtsStep user={user} onBack={() => setStep(3)} />}
    </WizardFrame>
  );
}
