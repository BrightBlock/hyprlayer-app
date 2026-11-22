interface Vars {
  status: "running" | "paused" | "ended";
  rpc: Record<string, string>;
  pauseAcked: boolean;
  stopAcked: boolean;
}

const gone = ["absent", "exited", "dead"];
const driving = (v: Vars) =>
  Object.values(v.rpc).some((pc) => !gone.includes(pc));

export const title = "A goal's run, pauses, stops and server restarts";

export const state = (v: Vars) => {
  const goal =
    v.status === "running"
      ? "Running"
      : v.status === "paused"
        ? "Paused"
        : "Ended";
  const who = driving(v) ? "a runner is on it" : "no runner";
  const asked = v.stopAcked
    ? ", you were told it is stopping"
    : v.pauseAcked
      ? ", you were told it will pause"
      : "";
  return `${goal}, ${who}${asked}`;
};

export const group = (v: Vars) => v.status;

export const colors = { running: "blue", paused: "orange", ended: "purple" };

const labels: Record<string, string> = {
  RunnerChain: "the new runner stops waiting for the old one",
  RunnerOpen: "the runner starts",
  RunnerResume: "the runner picks up after a restart",
  RunnerApply: "the runner takes your request",
  RunnerToGate: "the runner gets ready to send",
  RunnerGate: "the agent is sent the next iteration",
  RunnerTurnEnds: "the agent's turn ends; checks run",
  RunnerJudge: "the checks decide",
  RunnerFinishing: "the runner starts finishing",
  RunnerLock: "the runner takes the goal lock",
  RunnerDrain: "the runner reads its last requests",
  RunnerWrite: "the runner writes how the goal ends or pauses",
  RunnerLift: "the runner restores the agent's spend cap",
  RunnerExit: "the runner exits",
  PauseStart: "you press pause",
  PauseLock: "your pause takes the goal lock",
  PauseAct: "the server answers your pause",
  PauseWait: "your pause waits for the runner to finish",
  StopStart: "you press stop",
  StopLock: "your stop takes the goal lock",
  StopAct: "the server answers your stop",
  StopWait: "your stop waits for the runner to finish",
  ResumeStart: "you press resume",
  ResumeLock: "your resume takes the goal lock",
  ResumeAct: "the server answers your resume",
  ResumeSpawn: "a runner is started for your resume",
  Restart: "the server restarts",
  SrvLock: "the restarted server takes the goal lock",
  SrvAct: "the restarted server looks for running goals",
  SrvSpawn: "the restarted server starts a runner",
  WakerCheck: "the restarted server looks for cut-off sessions",
  WakerWake: "the session is woken",
  Done: "everything settles",
};

export const step = (action: string) => labels[action] ?? action;
