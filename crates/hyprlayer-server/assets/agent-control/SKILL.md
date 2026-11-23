---
name: orchestrate
description: Coordinate other hyprlayer agents. Use it to spawn a reviewer or implementer, hand work to another session, create a workstream, or approve or validate a plan.
---

# Orchestrating hyprlayer agents

You run inside hyprlayer. Its `hyprlayer` MCP server and the `"$HYPRLAYER_CTL" ctl <verb>` command reach the same tools. Start with `whoami`: it names your session or terminal, workstream, project and control level.

## Norms

- **Stay in your workstream.** Leave `workstream_id` out unless the user named another one. Create a workstream only for separate work on its own branch.
- **Spawn background agents.** A spawned agent opens as a background tab on the user's desktops and runs no higher than your own permission mode, in a stage that pre-approves no tool yours doesn't. Give it a self-contained prompt and a title.
- **Use `prompt_and_wait` rather than polling.** It sends, then blocks until the agent finishes its turn, needs a person, or exits, and returns the agent's last reply. Do not loop on `read_agent` or `list_agents`. A `stalled` result means an idle agent did not move; do not just call it again.
- **Use `wake_when` to wait on anything else, then end your turn.** Don't run shell `sleep` or poll loops, or call `pr_status`, `git_status` or `list_agents` again and again. hyprlayer sends you a message marked `[from hyprlayer · wake <id>]` when the condition holds or its timeout passes, and that starts your next turn.
- **Report to your spawner in your final reply.** `whoami`'s `parent` names the agent that spawned you, and its wait returns your last reply. To reach a session spawner mid-task, use `message_agent` with its `session_id`. That is refused when it runs in a higher permission mode than yours, and a terminal spawner can't be messaged at all. Claude's own `SendMessage` reaches no hyprlayer agent.
- **Never stop, kill or archive an agent you didn't spawn.** The user owns those.
- **Say why in `notify_user` and `request_attention`.** The user reads the body away from your transcript, so state the reason and what you need.

## Tools

Read tools work at agent control Read or Full:

| MCP tool | ctl |
|---|---|
| `whoami` | `ctl whoami` |
| `list_workstreams` | `ctl workstreams` |
| `list_agents` | `ctl agents` |
| `read_agent` | `ctl turns <session>` |
| `read_terminal` | `ctl read-terminal <terminal>` |
| `list_docs` | `ctl docs` |
| `read_doc` | `ctl doc <id>` |
| `pr_status` | `ctl pr` |
| `git_status` | `ctl git` |
| `script_output` | `ctl script-output <kind>` |
| `notify_user` | `ctl notify <title> <body>` |
| `request_attention` | `ctl attention <reason>` |
| `wake_when` | `ctl wake-when <condition-json>`; `--cancel <id>`, or no argument to list |

`pr_status` answers with a `kind`: `found` (the PR is in `pr`), `noPr`, `notGitHub` (no GitHub remote), `failed` (`cause` says why, `last` is the last known PR or null), or `unknown`.

These need Full. You get it when the user sets agent control to Full, or when you run in bypass permissions mode:

| MCP tool | ctl |
|---|---|
| `spawn_agent` | `ctl spawn-agent <prompt>`; `--engine <id>` starts a PTY agent |
| `wait_agent` | `ctl wait-agent <session>` |
| `message_agent` | `ctl message-agent <session> <text>` |
| `prompt_and_wait` | `ctl prompt-and-wait <session> <text>` |
| `stop_agent` | `ctl stop-agent <session>` |
| `kill_agent` | `ctl kill-agent <session>` |
| `archive_agent` | `ctl archive-agent <session>` |
| `answer_question` | `ctl answer <session> <request> <answers-json>` |
| `set_agent_title` | `ctl title <title>` |
| `create_workstream` | `ctl create-workstream <title>` |
| `approve_plan` | `ctl approve-plan <plan>` |
| `validate_plan` | `ctl validate-plan <plan>` |
| `start_goal` | `ctl start-goal <condition> <work-prompt> --checks <json>` |
| `stop_goal` | `ctl stop-goal <goal>` |
| `run_script` | `ctl run-script <kind>` |
| `stop_script` | `ctl stop-script` |
| `comment` | `ctl comment <path> <line> <body>` |
| `resolve_comment` | `ctl resolve-comment <comment>` |
| `reveal` | `ctl reveal doc <path>` (or `file`, `diff`) |

`--terminal <id>` in place of `<session>` targets a PTY agent. When the user runs app actions in a CLI, `spawn_agent`, `approve_plan` and `validate_plan` start one in the default engine and return its `terminalId`; pass `--harness` to keep a session. `"$HYPRLAYER_CTL" ctl --help` lists every flag; `--json` prints the raw result. ctl exits 0 on success, 1 on a refusal or no server, and 2 on a usage error.

## Refusals

A refusal carries a code, a message and a remedy. Don't retry it: change course, or tell the user.

- `agent_control_off`: the user turned agent control off. Nothing here works; carry on without it.
- `agent_control_denied`: the message says why:
  - the call needs Full and you are at Read;
  - the command is never open to agents: merging a PR, archiving or deleting a workstream, running the archive script, discarding changes, removing a project, creating a global document, answering permission prompts, changing a permission mode or settings, skills, agent definitions, the agent bundle, the Cloud account;
  - the target is not an agent you spawned;
  - the target belongs to another project, or is a document outside it you tried to write or attach;
  - you are a report-only session;
  - you are a codex agent outside bypass mode, which only reads;
  - the target is waiting on a permission prompt or a question, which a message would answer;
  - a child would run above your own permission mode, or in a stage that pre-approves tools yours doesn't;
  - a goal has a command check and you are not in bypass mode;
  - the session you message runs in a higher permission mode than yours and is not one you spawned;
  - the PTY engine is not one the user configured.
- `agent_limit_reached`: a child would be more than 2 agents deep, you already have 4 live children, or an agent got 6 agent messages with no human turn in between. Ask the user to step in; they can raise these limits in Settings → Agent control. It also comes when you already have 8 pending wakes; cancel one first. A wake counts as an agent message, so one past the hop limit is dropped.
