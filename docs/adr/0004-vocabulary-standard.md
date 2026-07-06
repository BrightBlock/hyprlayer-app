# ADR 0004: Vocabulary standard — typed enums and naming glossary

**Status:** accepted · 2026-07-05

## Decision

All closed-vocabulary domain types (Stage, SessionState, WorktreeMode, Harness,
PermissionMode, DocKind) are defined as Rust enums with `serde` + `specta` in
`crates/hyprlayer-core`, exported to the frontend via generated bindings. The
frontend must not re-declare these types; it derives them from `src/lib/bindings.ts`
and re-exports them through `src/lib/domain/`.

### Glossary

| Term | Definition |
|---|---|
| **agent** | UI umbrella term for an LLM conversation session |
| **harness** | Chat runtime driven via the `AgentDriver` trait (claude, opencode) |
| **engine** | Opaque CLI binary the multiplexer runs in a PTY |
| **provider** | Model vendor (Anthropic, OpenAI, etc.) |

### Naming rules

- IPC commands: `domain_verb` (e.g. `create_workstream`, `start_stage_session`)
- PTY commands: `pty_*` (e.g. `pty_agent_write`, `pty_agent_resize`) — not `agent_*`
- SQLite columns stay TEXT; core `parse()` functions do tolerant parsing so
  legacy rows keep loading without a migration
- Cross-layer identifiers use `harness`, not harness-specific names
  (`harness_session_id`, not `claude_session_id`)

### Frontend discipline

| If a type is... | Put it in... |
|---|---|
| A shared vocabulary enum (Stage, Harness, DocKind, PermissionMode) | `src/lib/domain/` — derived from bindings, never re-declared |
| Feature-owned domain data (e.g. ShellTab, WsTabState) | The feature that owns its persistence |
| Used by two or more features | Graduate to `src/lib/domain/` or `src/lib/` |

## Rationale

Before this decision, Stage was declared 5× across the codebase (Rust enum, String +
comment in two store modules, TS re-derivation, command-site re-validation).
Harness was declared 5× with the two TS declarations already disagreeing (one
included codex, the other didn't). String comparisons gated destructive behavior
(`is_inplace()` matched raw `"inplace"`). The glossary terms had no documented
hierarchy, and `TerminalEngine` meant two unrelated things.

A single source of truth in core, exported through specta bindings, eliminates
the drift and makes the vocabulary machine-checkable. Tolerant parsing keeps
the TEXT columns backward-compatible.

## Consequences

- Adding a new stage/harness/mode variant requires updating the Rust enum,
  regenerating bindings (`cargo test`), and updating any tolerant-parser to
  accept the new string. No frontend re-declaration needed.
- The frontend `lib/domain/` directory is the authority on what vocabulary is
  shared. Adding a file there is an explicit signal that the type is export-grade.
- Renaming `claude_session_id` → `harness_session_id` requires a DB migration
  (column rename + event field rename) — deferred to the harness-surface layering
  phase.