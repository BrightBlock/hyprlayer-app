# Failed start and Resume

When a session can't launch, the Agent tab keeps the prompt and shows an exit card, `Failed to start session`, with the reason, then the `Session ended.` bar with `New session` and `Resume`. The model, effort and permission pickers stay usable, so the user can fix the cause and press Resume. A start that never launched isn't tied to a provider: every harness's models are enabled. Resume retries the same session on what the pickers show, moving it to another harness if one was picked, and replays the stored first turn. A Resume that fails again adds one more exit card and stays on the failed view.

## Sub-features

- `fs-view` shows the user row, the `Failed to start session` card with the reason, and the ended bar. The session row is `spawn_failed`. <!-- src: src/features/session/transcript.ts:1084 --> <!-- src: crates/hyprlayer-core/src/runtime/sessions/service.rs:1475 -->
- `fs-pickers` keeps the status bar's pickers enabled on a failed start. <!-- src: src/features/session/SessionChat.tsx:1318 -->
- `fs-harness` unlocks every provider for a start with no `init`, and Resume moves it to the picked harness through `set_session_harness`, which also replaces the row's model and effort. The replayed first turn is the stage prompt for the new harness. A session that ran keeps its provider. <!-- src: src/features/session/SessionChat.tsx:415 --> <!-- src: src/features/session/SessionChat.tsx:515 --> <!-- src: crates/hyprlayer-core/src/runtime/sessions/service.rs:457 --> <!-- src: crates/hyprlayer-core/src/pipeline.rs:432 -->
- `fs-refused` handles a Resume that fails again: one more exit card, no separate error banner. <!-- src: src/features/session/SessionChat.tsx:1441 --> <!-- src: src/features/session/transcript.ts:278 -->
- `fs-retry` handles a Resume that launches: it replays the stored first turn, and the turn timer starts at the retry. <!-- src: src/features/session/SessionChat.tsx:1476 --> <!-- src: src/features/session/sessionStore.ts:1416 -->
- `fs-no-auto` means a failed start never resumes on its own. <!-- src: src/features/session/transcript.ts:269 -->

## How to get to it (user POV)

- Start a session whose harness can't launch, e.g. a missing binary. The tab shows the failed view.
- Fix the cause (Settings › Binaries `Reset`, or pick another model), then click `Resume` in the ended bar.

## Driving it with the MCP bridge

Preconditions:

- **Isolated** instance with a fresh folder workstream on its unstarted Agent tab. Turn off tab titles and recaps (session-send.md seed) so the retry spends only its own turn.
- **Seed a missing claude** (`Locate…` opens a native picker): `invoke('set_binary_override', { kind: 'claude', path: '<run-dir>/seed/no-such-claude' })`, then reload. `get_bootstrap().binaries` shows claude `missing`. <!-- src: crates/hyprlayer-core/src/runtime/settings_service.rs:156 -->
- **Cost:** the failure and the refused Resume are $0. The good Resume is one short Haiku turn (about $0.012).

- **Start.** Pick a Haiku model in the status bar, type `Reply with exactly VERIFY-RETRY-1` and click `Start`. Within ~3s the pane shows the prompt, `Failed to start session`, ``required binary `claude` was not found`` and `Session ended.` with `New session` and `Resume`. `list_sessions` (`{ workstreamId }`) has one row, `state: "spawn_failed"`, with that reason as `exitSummary`. <!-- src: crates/hyprlayer-core/src/error.rs:23 -->
- **Pickers.** `button[aria-label^="Model: "]`, `Effort: ` and `Permission mode: ` are not `disabled`.
- **Refused Resume.** Click `Resume` (`//button[normalize-space()="Resume"]`) with claude still missing. The pane now shows two `Failed to start session` cards and no separate banner, and the row is still `spawn_failed`. `get_session_log` kinds read `userMessage,exited,exited`.
- **Fix.** Gear › `All settings` › `Binaries`, then the claude card's `Reset`. `get_bootstrap` shows claude `ok`, and `get_app_settings().binaryOverrides` is `{}`.
- **Resume.** Back on the tab, click `Resume`. The reply streams under the original prompt, and the activity line reads a few seconds (`Cooked for 3.5s`), timed from the retry. The log gains `relaunched`, `init` and `turnDone`. `ps -o args= -p <claude pid>` shows `--session-id <session id>`; the pid is in the server log's `started as pid` line. <!-- src: crates/hyprlayer-core/src/claude/driver.rs:159 -->
- **Another harness.** Seed a failed OpenCode start the same way (override `opencode` to a missing path, start on a free OpenCode model). Open the model menu: the Claude and Codex rows are enabled. Pick `Haiku` and click `Resume`. The reply comes from claude, and the `list_sessions` row reads `harness: "claude"` with the Haiku model. A Claude pick writes nothing until Resume, so the row keeps its OpenCode model while you choose. `set_session_harness` on a session that has run answers `session_launched`. <!-- src: src/features/session/sessionStore.ts:1416 --> <!-- src: src/features/session/SessionChat.tsx:613 -->
- **Proof.** Screenshot the failed view and the replayed reply. Save the `list_sessions` row before and after, the log kinds, and the `ps` line.

## Gotchas

- Raw IPC skips the bootstrap cache, so reload after seeding the override.
- The provider lock is gone only for a start that never got an `init`. A session whose relaunch failed after it ran (`spawn_failed` with an `init` in its log) still reads `Locked to this session's provider` on other providers' rows. <!-- src: src/features/session/SessionChat.tsx:515 --> <!-- src: src/components/OptionMenuList.tsx:28 -->
- The model menu never lists `github-copilot/*` OpenCode models, so they can't be used to trigger a failure. <!-- src: src/lib/domain/harnessing.ts:32 -->
- Reset the override before cleanup. Isolated cleanup removes the profile anyway.
