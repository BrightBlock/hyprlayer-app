import type { CreateStep, ProjectCreateProgress, StepStatus } from "@/lib/bindings";
import { Spinner, cx } from "@/components/ui";
import { CheckIcon, XIcon } from "@/components/icons";

export type StepState = { status: StepStatus; detail: string | null };
export type ProgressMap = Partial<Record<CreateStep, StepState>>;

export function applyProgress(prev: ProgressMap, event: ProjectCreateProgress): ProgressMap {
  return { ...prev, [event.step]: { status: event.status, detail: event.detail } };
}

const LABELS: Record<CreateStep, { idle: string; running: string; done: string }> = {
  folder: {
    idle: "Create the project folder",
    running: "Creating the project folder",
    done: "Created the project folder",
  },
  repository: {
    idle: "Initialize the repository",
    running: "Initializing the repository",
    done: "Initialized the repository",
  },
  commit: {
    idle: "Make the first commit",
    running: "Making the first commit",
    done: "Made the first commit",
  },
  githubRepo: {
    idle: "Create the GitHub repository",
    running: "Creating the GitHub repository",
    done: "Created the GitHub repository",
  },
  push: {
    idle: "Push to origin",
    running: "Pushing to origin",
    done: "Pushed to origin",
  },
  thoughts: {
    idle: "Link the thoughts folder",
    running: "Linking the thoughts folder",
    done: "Linked the thoughts folder",
  },
};

const LOCAL_STEPS: CreateStep[] = ["folder", "repository", "commit", "thoughts"];
const GITHUB_STEPS: CreateStep[] = ["githubRepo", "push"];

export function CreateProgressPanel({
  progress,
  withGithub,
}: {
  progress: ProgressMap;
  withGithub: boolean;
}) {
  const steps = [...LOCAL_STEPS, ...(withGithub ? GITHUB_STEPS : [])];
  return (
    <div className="divide-y divide-edge/60 rounded-md border border-edge bg-panel-2/40">
      {steps.map((step) => (
        <Row key={step} step={step} state={progress[step]} />
      ))}
    </div>
  );
}

function Row({ step, state }: { step: CreateStep; state: StepState | undefined }) {
  const status = state?.status;
  const label =
    status === "done"
      ? LABELS[step].done
      : status === "running"
        ? LABELS[step].running
        : LABELS[step].idle;
  const detail = status === "failed" ? "failed" : status === "skipped" ? "skipped" : state?.detail;

  return (
    <div className="flex h-9 items-center gap-2.5 px-3">
      <span className="flex h-4 w-4 shrink-0 items-center justify-center">
        {status === "done" ? (
          <CheckIcon size={13} className="text-ok" />
        ) : status === "running" ? (
          <Spinner size={14} />
        ) : status === "failed" ? (
          <span className="text-bad">
            <XIcon size={12} />
          </span>
        ) : (
          <span aria-hidden="true" className="h-[5px] w-[5px] rounded-full bg-edge" />
        )}
      </span>
      <span
        className={cx(
          "min-w-0 flex-1 truncate text-body",
          status === "running" || status === "failed" ? "text-ink" : "text-ink-dim",
          status === undefined || status === "skipped" ? "opacity-60" : null,
        )}
      >
        {label}
      </span>
      {detail && (
        <span
          className={cx(
            "shrink-0 truncate text-ui",
            status === "failed" ? "font-mono text-bad" : "font-mono text-ink-dim",
            status === "skipped" && "font-sans opacity-60",
          )}
          style={{ maxWidth: 190 }}
        >
          {detail}
        </span>
      )}
    </div>
  );
}
