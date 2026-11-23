# hyprlayer-core vocab & naming conventions

## Glossary

| Term | Means |
|---|---|
| **agent** | UI umbrella term for an LLM conversation |
| **harness** | chat runtime driven via `AgentDriver` trait (claude, codex, opencode) |
| **engine** | opaque CLI binary the multiplexer runs in a PTY |
| **provider** | model vendor (Anthropic, OpenAI, etc.) |

## Typed vocabulary (never re-declare string literals)

All closed-vocabulary types live in `crates/hyprlayer-core` with `serde` + `specta`.
The frontend gets them from `src/lib/bindings.ts`. **Never re-declare these as string
enums or raw `string` types** — use the generated binding.

| Type | Core location | Frontend binding |
|---|---|---|
| `Stage` | `crates/hyprlayer-core/src/pipeline.rs` | Binding re-exported from `lib/domain/stages.ts` |
| `SessionState` | `crates/hyprlayer-core/src/store/sessions.rs` | Binding re-exported from `lib/domain/stages.ts` |
| `WorktreeMode` | `crates/hyprlayer-core/src/worktree/mod.rs` | Direct binding via specta |
| `WorkstreamStatus` | `crates/hyprlayer-core/src/store/workstreams.rs` | Direct binding via specta |
| `Harness` | `crates/hyprlayer-core/src/agent.rs` | `lib/domain/harnessing.ts` |
| `PermissionMode` | `crates/hyprlayer-core/src/claude/driver.rs` | `lib/domain/permissions.ts` |
| `DocType` / `DocStatus` / `DocScope` | `crates/hyprlayer-core/src/docs/vocab.rs` | `lib/domain/docs.ts` |
| `GoalStatus` / `GoalStep` / `GoalPauseReason` / `GoalEndReason` / `GoalSource` | `crates/hyprlayer-models/src/goal.rs` | `lib/domain/goals.ts` |
| `SessionOrigin` | `crates/hyprlayer-models/src/session.rs` | `lib/domain/agentControl.ts` |
| `AgentControlLevel` / `PtyAgentState` | `crates/hyprlayer-models/src/agent_control.rs` | `lib/domain/agentControl.ts` |
| `LeadRuleScope` | `crates/hyprlayer-core/src/settings.rs` | Direct binding via specta |
| `LeadQueueState` | `crates/hyprlayer-models/src/agent_control.rs` | Direct binding via specta |

SQLite columns stay TEXT. Core `parse()` functions do tolerant parsing so legacy
rows keep loading.

## IPC surface

- Command naming: `domain_verb` (`create_workstream`, `start_stage_session`)
- Unused: `agent_*` prefix — the convention is `domain_verb`. PTY commands use `pty_*`.
- Every IPC command returns the `AppError` envelope (typed error with code + remedy)
- tauri-specta generate events are kebab-cased; the frontend bridge maps to camelCase

## Avoid these

- `claude_session_id` — it stores the active harness session id, not just Claude. Renaming to `harness_session_id` is pending.
- `TerminalEngine` — "engine" means an opaque CLI binary; the frontend `TerminalEngine` is a terminal emulator.
- Duplicating `basename`/`dirname` — use `src/lib/path.ts` (the one canonical implementation).

See `docs/adr/0004-vocabulary-standard.md` for the full decision.