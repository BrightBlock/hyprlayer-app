# Failed start and Resume

When a session can't launch, the Agent tab keeps the prompt and shows an exit card, `Failed to start session`, with the reason, then the `Session ended.` bar with `New session` and `Resume`. The model, effort and permission pickers stay usable, so the user can fix the cause and press Resume. Resume retries the same session on what the pickers show and replays the stored first turn. A Resume that fails again adds one more exit card and stays on the failed view.

## Sub-features

- `fs-view` shows the user row, the `Failed to start session` card with the reason, and the ended bar. The session row is `spawn_failed`. <!-- src: src/features/session/transcript.ts:1040 --> <!-- src: crates/hyprlayer-core/src/runtime/sessions/service.rs:1408 -->
- `fs-pickers` keeps the status bar's pickers enabled on a failed start. <!-- src: src/features/session/SessionChat.tsx:1513 -->
- `fs-refused` handles a Resume that fails again: one more exit card, no separate error banner. <!-- src: src/features/session/SessionChat.tsx:1634 --> <!-- src: src/features/session/transcript.ts:276 -->
- `fs-retry` handles a Resume that launches: it replays the stored first turn, and the turn timer starts at the retry. <!-- src: src/features/session/SessionChat.tsx:1667 --> <!-- src: src/features/session/sessionStore.ts:1144 -->
- `fs-no-auto` means a failed start never resumes on its own. <!-- src: src/features/session/transcript.ts:267 -->

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
- **Resume.** Back on the tab, click `Resume`. The reply streams under the original prompt, and the activity line reads a few seconds (`Cooked for 3.5s`), timed from the retry. The log gains `relaunched`, `init` and `turnDone`. `ps -o args= -p <claude pid>` shows `--session-id <session id>`; the pid is in the server log's `started as pid` line. <!-- src: crates/hyprlayer-core/src/claude/driver.rs:164 -->
- **Proof.** Screenshot the failed view and the replayed reply. Save the `list_sessions` row before and after, the log kinds, and the `ps` line.

## Gotchas

- Raw IPC skips the bootstrap cache, so reload after seeding the override.
- The pickers stay usable, but they're still locked to the failed session's provider. After a failed OpenCode start, only that provider's models are enabled; Claude and Codex rows read `Locked to this session's provider. Start a new agent to use a different provider`. A provider-level failure can only be left through `New session`. <!-- src: src/features/session/SessionChat.tsx:726 --> <!-- src: src/lib/domain/harnessing.ts:223 --> <!-- src: src/components/OptionMenuList.tsx:28 -->
- The model menu never lists `github-copilot/*` OpenCode models, so they can't be used to trigger a failure. <!-- src: src/lib/domain/harnessing.ts:32 -->
- Reset the override before cleanup. Isolated cleanup removes the profile anyway.
