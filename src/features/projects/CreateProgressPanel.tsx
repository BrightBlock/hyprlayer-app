import type { CreateStep, ProjectCreateProgress, StepStatus } from "@/lib/bindings";
import { StepList, StepRow } from "@/components/ui";

type StepState = { status: StepStatus; detail: string | null };
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
    <StepList>
      {steps.map((step) => {
        const state = progress[step];
        const label =
          state?.status === "done"
            ? LABELS[step].done
            : state?.status === "running"
              ? LABELS[step].running
              : LABELS[step].idle;
        return <StepRow key={step} label={label} status={state?.status} detail={state?.detail} />;
      })}
    </StepList>
  );
}
