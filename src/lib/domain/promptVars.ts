
import type { PromptContext } from "@/lib/bindings";

export const PROMPT_VARS: ReadonlyArray<{
  token: string;
  key: keyof PromptContext;
  desc: string;
}> = [
  { token: "{{branch}}", key: "branch", desc: "Current worktree branch" },
  { token: "{{base_branch}}", key: "baseBranch", desc: "PR target branch" },
  { token: "{{changed_files}}", key: "changedFiles", desc: "Working-tree change summary" },
  { token: "{{diff}}", key: "diff", desc: "Working-tree diff vs HEAD (truncated)" },
  { token: "{{last_commit}}", key: "lastCommit", desc: "Most recent commit subject" },
];
