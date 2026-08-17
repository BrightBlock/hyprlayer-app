/**
 * The `{{variable}}` tokens a prompt body may reference, resolved from the
 * focused workstream's worktree when the prompt runs
 * (`workstream_prompt_context`). Shared vocabulary: the prompt library offers
 * them in its editor, the agent library in its system-prompt editor.
 */

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
