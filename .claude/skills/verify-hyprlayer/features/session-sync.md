# Sync notices across clients

Two desktops attached to one server can both show the same session. Turns another client sends aren't applied live: the composer shows a strip, `N new turns from <device>`, with `Load` and a dismiss `×`. If the user has a draft when they arrive, a warning card replaces the strip: `This session moved on from another client`, with `Pull N turns` and `Send anyway`, and the send button waits. Pulling inserts the turns under a `New from <device>` divider and flashes `Synced with <device>`. The draft stays.

## Sub-features

- `sync-behind` shows the `N new turns from <device>` strip, with `Load` and `Dismiss`. <!-- src: src/features/session/SyncComposerNotice.tsx:120 --> <!-- src: src/features/session/SyncComposerNotice.tsx:130 -->
- `sync-load` pulls the held turns in, under a `New from <device>` divider, then shows `Synced with <device>`. <!-- src: src/features/session/SyncComposerNotice.tsx:160 -->
- `sync-guard` shows the warning card when a draft exists, and holds Send. <!-- src: src/features/session/SyncComposerNotice.tsx:168 --> <!-- src: src/features/session/SyncComposerNotice.tsx:59 --> <!-- src: src/features/session/SyncComposerNotice.tsx:187 -->
- `sync-route` holds another client's turns; agent-authored turns apply live. <!-- src: src/features/session/transcriptSync.ts:261 --> <!-- src: src/features/session/transcriptSync.ts:266 -->

## How to get to it (user POV)

- Open the same session on two desktops connected to one server (a remote host, or a second app sharing a local profile).
- On the second desktop, open the session from the status bar's `Agent sessions` popover and send a message. The first desktop's composer shows the notice. <!-- src: src/features/session/AgentSessionsButton.tsx:39 -->

## Driving it with the MCP bridge

Preconditions:

- **App A:** a doctor-passed isolated instance with a folder workstream. Start a short Haiku session there (`Reply with exactly SYNC-A-1`), titles and recaps off (session-send.md seed).
- **App B:** a second debug app attached to A's server, never a second server on that profile. Launch it as review-workspace.md's sandbox steps 3–4 do, with Vite on the next free port (1432) and `localProfile` set to A's `~/Library/Application Support/<A IDENTIFIER>`. <!-- src: src-tauri/src/hosts.rs:135 --> <!-- src: src-tauri/src/supervisor.rs:226 -->
  - Ready is `connected to <adjective-animal>` in B's `dev.log`, and `announced this client as <device>; transcript sync on`.
  - B's doctor fails its last two checks by design, as for the sandbox. `lsof -U | grep <A IDENTIFIER>/hyprlayer.sock` shows only A's server.
  - B's bridge takes the next port (9224). Drive each app with `appIdentifier` set to its port.
- **Cost:** one Haiku turn per B send, about $0.006 each on a short session.

- **Open in B.** Click the workstream, then `button[aria-label="Agent sessions"]`, then the session row's title in the popover. A new tab named after the first prompt opens on A's transcript.
- **Behind.** With A's composer empty, send `Reply with exactly SYNC-B-1` from B. Within ~5s A's composer reads `2 new turns from <device> · just now` with `Load`, and A's transcript doesn't show B's turn yet.
- **Load.** In A, click `Load` (`//*[@data-pane-content]//button[normalize-space()="Load"]`). B's turn appears under `New from <device>`, and the notice reads `Synced with <device>`.
- **Guard.** Type a draft into A's composer without sending, then send `Reply with exactly SYNC-B-2` from B. A shows the warning card: `This session moved on from another client`, `<device> added 2 turns while you were drafting.`, `Pull 2 turns`, `Send anyway`. `button[aria-label="Send"]` is `disabled`.
- **Pull.** Click `Pull 2 turns`. B's turn is in A's transcript, the draft is unchanged, and Send is enabled again.
- **Read back.** `get_session_log` (`{ sessionId }`) lists each `userMessage` once, in order.
- **Teardown.** `driver_session stop` for B's port, then `verify.sh cleanup <B run-dir>`. It removes only B's profile dir, and A keeps running. Doctor A afterwards.
- **Proof.** Screenshot A's strip, the guard card and the synced transcript. Save the log.

## Gotchas

- The device label is the machine name, so both apps on one Mac read the same `<device>`.
- In B, the status bar shows B's own default model and permission (`Opus 5.5`, `Auto`) for the adopted session, not the session's model. Read the model from `list_active_sessions`.
- After a pull, A's activity line reads `… for 0.0s` for the pulled turn.
- An ended view that missed a revision catches up on the watch poll (#418); this recipe doesn't drive it.
