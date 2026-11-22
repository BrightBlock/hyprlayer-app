# Hyprlayer Cloud account

The account button at the header's right end shows the Hyprlayer mark while signed out and the user's initials while signed in. Its popover offers `Sign in with GitHub`. That opens the system browser on Cognito's hosted UI, and the app waits (`Waiting for GitHub…`, with Cancel) for the redirect to a desktop loopback port. Once signed in, it shows the name, the email and `Sign out`. Per ADR-0011 the server holds the session in `account.json`. A debug build always signs in to the dev pool.

## Sub-features

- `acct-button`: the `aria-label` is `Sign in` when signed out and `Account: <name or email>` when signed in. It needs a connected server.
- `acct-popover`: a `[role="dialog"]` labelled `Account`.
- `acct-signin-wait`: Sign in binds a loopback, opens the browser and shows `Waiting for GitHub…`.
- `acct-signin-cancel`: Cancel ends the attempt with `account_sign_in_cancelled` and no error note.
- `acct-signed-in`: name, email, and `Sign out`.
- `acct-signout`: revokes the refresh token and deletes `account.json`.

## How to get to it (user POV)

- Click the account button, the rightmost header control, after the divider.

## Driving it with the MCP bridge

Preconditions:

- An isolated instance, connected (seed optional). Its server has no `account.json`, so it reads as signed out. The file would live at `~/Library/Application Support/<IDENTIFIER>/account.json`.
- Not drivable end to end. Completing sign-in needs a human GitHub login in the real browser, and the bridge can't reach the browser. `crates/hyprlayer-e2e` scenario `account_sign_in` covers the exchange against a stub (golden `account_sign_in.json`).
- Cost and risk: clicking Sign in opens a tab in the user's default browser at the dev pool's login domain. Tell the user before you click.
- Loopback ports are fixed at 1430 and 1420, minus this instance's own Vite port. A shared `.dev` instance's Vite holds 1430, so the attempt binds 1420. If both are busy, sign-in fails with `account_ports_busy`. <!-- src: src-tauri/src/commands/account.rs:15 --> <!-- src: src-tauri/src/commands/account.rs:100 --> <!-- src: crates/hyprlayer-core/src/error.rs:947 -->

- **Read state.** Run `invoke('account_status')`, which returns `{ state: "signedOut" }`. `webview_find_element` `button[aria-haspopup="dialog"][aria-label="Sign in"]` matches. <!-- src: src/lib/bindings.ts:212 --> <!-- src: src/lib/bindings.ts:395 --> <!-- src: src/features/account/AccountButton.tsx:46 --> <!-- src: src/features/account/AccountButton.tsx:38 -->
- **Open.** Click that button. A `[role="dialog"]` with heading `Account` shows one row, `Sign in with GitHub`. <!-- src: src/features/account/AccountPopover.tsx:43 --> <!-- src: src/features/account/AccountPopover.tsx:108 -->
- **Start (opens the browser).** Click the popover's row with XPath `//div[@role="dialog"]//button[normalize-space()="Sign in with GitHub"]`. A text match also hits the header button, whose `title` is the same. The row becomes a spinner, `Waiting for GitHub…`, and a `Cancel` button. `dev.log` in the run dir logs `account sign-in listening on port <1430|1420>`. <!-- src: src/features/account/AccountPopover.tsx:99 --> <!-- src: src/features/account/AccountPopover.tsx:101 --> <!-- src: src-tauri/src/commands/account.rs:105 -->
- **Cancel.** Click `Cancel` inside the popover. It returns to `Sign in with GitHub` with no error note, since a cancel isn't a failure. `dev.log` logs `account sign-in ended: account_sign_in_cancelled`. `account_status` is still `signedOut`. The browser tab stays on the hosted UI; close it by hand. <!-- src: src/features/account/AccountPopover.tsx:116 --> <!-- src: src-tauri/src/commands/account.rs:81 --> <!-- src: crates/hyprlayer-core/src/error.rs:946 -->
- **Signed in (shared, read-only).** If the user is already signed in on `.dev`, the button's `aria-label` is `Account: <name>`. The popover shows the name, the email and `Sign out`. Screenshot it and stop. <!-- src: src/features/account/AccountPopover.tsx:75 --> <!-- src: src/features/account/AccountPopover.tsx:85 -->
- **Proof.** Save screenshots of the waiting state and the restored state, the `account_status` JSON, and the two `dev.log` lines (`grep 'account sign-in' <run-dir>/dev.log`).

## Gotchas

- Never click `Sign out` in a shared profile. It revokes the refresh token, and getting it back needs the user's browser sign-in.
- Never call raw `account_sign_in` as a read-back. It opens the browser. Use `account_status`. <!-- src: src/lib/bindings.ts:213 -->
- The popover closes and won't open while the server is disconnected. <!-- src: src/features/account/AccountButton.tsx:25 -->
- An unanswered attempt times out after 300 s, and starting a new one supersedes the old. <!-- src: src-tauri/src/commands/account.rs:16 --> <!-- src: src-tauri/src/commands/account.rs:25 -->
- A release `account.json` (prod pool) next to a debug server reads as signed out. Pools aren't shared. <!-- src: docs/adr/0011-cloud-account.md:44 -->
- `account_status` is cached in react-query for 60 s. The server's `AccountChanged` event refreshes it. Raw IPC doesn't. <!-- src: src/lib/queries.ts:622 --> <!-- src: src/lib/events.ts:123 -->
