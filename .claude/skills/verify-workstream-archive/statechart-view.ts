interface Vars {
  hold: Record<string, string>;
  ext: boolean;
  pc: string;
  fails: number;
}

export const title = "Archiving a workstream while things still hold its worktree open";

export const state = (v: Vars) => {
  const open = Object.entries(v.hold)
    .filter(([, s]) => s !== "free")
    .map(([h]) => h);
  if (v.ext) open.push("an outside process");
  const held = open.length ? ` (open: ${open.join(", ")})` : "";
  const phase: Record<string, string> = {
    idle: "Active",
    stopping: "Stopping its processes",
    moving: v.fails ? `Moving, ${v.fails} refused` : "Moving",
    archived: "Archived",
    rolledback: "Not archived, rolled back",
  };
  return `${phase[v.pc] ?? v.pc}${held}`;
};

export const group = (v: Vars) =>
  v.pc === "archived" ? "done" : v.pc === "rolledback" ? "failed" : "busy";

export const colors = { done: "green", failed: "red", busy: "orange" };

const labels: Record<string, string> = {
  Start: "you archive, and everything in the worktree is asked to stop",
  Released: "one holder's handles close",
  Stopped: "archive goes on to the move",
  ExternalOpens: "an indexer or virus scanner opens a file in the tree",
  ExternalCloses: "it closes the file",
  Move: "git worktree move runs",
  Done: "nothing more happens",
};

export const step = (action: string) => labels[action] ?? action;
