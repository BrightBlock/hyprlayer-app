# Servers page

Settings → Servers lists This machine plus every saved SSH server. Each row shows a state dot, a `DEFAULT` tag and a detail line. The user adds a server by SSH destination and label, which saves `hosts.json` and runs an `ssh` test at once. Rows can be edited, removed, retried and tested all together. Set default switches the app to that server and restarts it. Install puts a Hyprlayer server on a host that lacks one.

## Sub-features

- `srv-list`: `ul[aria-label="Servers"]` shows the `This machine` row (`DEFAULT` when `active` is null) and the saved hosts.
- `srv-validate`: inline alerts for spaces, a leading `-`, a URL, the reserved `local`, or a duplicate destination.
- `srv-add`: Add server writes `hosts.json` and probes the host.
- `srv-test`: Retry on one row and `Test all`. An unreachable row shows a warn dot and `<dest> · <reason>`.
- `srv-edit`: inline destination and label editor, with Save, Cancel and Escape.
- `srv-remove`: the `Remove server?` confirm, which removes the host from `hosts.json`.
- `srv-switch`: `Set default` opens `Switch server?`. Confirming writes `active` and relaunches the app.
- `srv-install`: `Install` appears only on a reachable host that has no server (`remote_cli_missing`).

## How to get to it (user POV)

- Settings popover, then `Servers`.
- `All settings`, then `Servers` in the nav.
- Works while disconnected; the page is desktop-local.

## Driving it with the MCP bridge

Preconditions:

- Use an **isolated** instance. Seeding is optional. Its `hosts.json` is `~/Library/Application Support/<IDENTIFIER>/hosts.json`, absent until the first save, and cleanup removes it. <!-- src: src-tauri/src/hosts.rs:84 -->
- Shared mode writes the user's real `hosts.json`. Copy it to evidence first, remove anything you add through `Remove`, and `diff` the file against the copy at the end. Never `Set default` there.
- The test runs real `ssh <destination>` with `~/.ssh/config`. Use an RFC 2606 `.invalid` name so it fails at DNS without a password or host-key prompt.
- Switch and Install need a real reachable SSH host. Without one, report them as unreachable entry points. `crates/hyprlayer-e2e` `remote_host` covers that path against a container.

- **Open.** Go to `/settings?section=servers`. The crumb reads `Settings / Servers`, and the header meta reads `1 server` on a fresh profile. <!-- src: src/features/settings/SettingsHeader.tsx:30 --> <!-- src: src/features/settings/settingsMeta.ts:16 -->
- **List.** With the status-bar server menu closed, run `webview_find_element` on `ul[aria-label="Servers"] > li`. There is one row, `This machine`, with `DEFAULT` and a `[role="img"][aria-label="Connected"]` dot. <!-- src: src/features/hosts/ServersSettings.tsx:206 --> <!-- src: src/features/hosts/ServerSummary.tsx:67 --> <!-- src: src/features/hosts/ServerSummary.tsx:77 --> <!-- src: src/lib/domain/hosts.ts:99 -->
- **Validate.** `webview_keyboard` `type` into `input[aria-label="SSH destination"]` the text `bad host`. A `[role="alert"]` reads `Destination cannot contain spaces.`, and `Add server` is disabled. Type `local` over it; the alert reads `Destination cannot be "local" — that name is this machine.` <!-- src: src/features/hosts/ServersSettings.tsx:167 --> <!-- src: src/features/hosts/ServersSettings.tsx:187 --> <!-- src: src/lib/domain/hosts.ts:16 --> <!-- src: src/lib/domain/hosts.ts:22 -->
- **Add.** Type `verify-nohost.invalid` in `SSH destination` and `Verify bogus` in `input[aria-label="Label"]`, then click `form button[type="submit"]` (`Add server`). The fields clear, and a `Verify bogus` row appears with a spinner, then a warn dot labelled `Unreachable` and detail `verify-nohost.invalid · Unreachable`, plus a `Retry` button. The nav's Servers count becomes `2`. <!-- src: src/features/hosts/ServersSettings.tsx:120 --> <!-- src: src/features/hosts/ServersSettings.tsx:182 --> <!-- src: src/features/hosts/ServerSummary.tsx:48 --> <!-- src: src/features/hosts/failures.ts:9 --> <!-- src: src/features/hosts/ServersSettings.tsx:373 --> <!-- src: src-tauri/src/supervisor.rs:631 -->
- **Read back.** Run `invoke('list_hosts')`, which returns `{ hosts: [{ destination: 'verify-nohost.invalid', label: 'Verify bogus' }], active: null, … }`. Then `cat` the `hosts.json`. <!-- src: src/lib/bindings.ts:298 -->
- **Duplicate.** Type `verify-nohost.invalid` again. The alert reads `Another server already uses this destination.` The bridge can't empty the field, so leave it or type over it. <!-- src: src/features/hosts/ServersSettings.tsx:40 -->
- **Test all / Retry.** Click `Test all` or the row's `Retry`. The dot spins and returns to `Unreachable`. `Test all` skips the connected row and is disabled when nothing is testable. <!-- src: src/features/hosts/ServersSettings.tsx:102 --> <!-- src: src/features/hosts/ServersSettings.tsx:200 -->
- **Edit.** Click `button[aria-label="Edit"]`. Only the bogus row has one, because This machine has no host. Type into `input[aria-label="Label for Verify bogus"]` to make it `Verify renamed`, then click `Save`. `list_hosts` shows the new label. Escape in either field cancels. <!-- src: src/features/hosts/ServersSettings.tsx:384 --> <!-- src: src/features/hosts/ServersSettings.tsx:444 --> <!-- src: src/features/hosts/ServersSettings.tsx:414 -->
- **Remove.** Click `button[aria-label="Remove"]`. A `[role="dialog"]` titled `Remove server?` names the host. Click `Remove` (`strategy: "text"`, the dialog's button). The row is gone, and `list_hosts` returns `hosts: []`. <!-- src: src/features/hosts/ServersSettings.tsx:386 --> <!-- src: src/features/hosts/ServersSettings.tsx:255 --> <!-- src: src/features/hosts/ServersSettings.tsx:138 -->
- **Switch (real host only).** `Set default` (`button[aria-label="Set default"]`) shows only on a reachable, non-default row. It opens `Switch server?`, with `Hyprlayer will restart on <name>.` and a `Switch and restart` button. Click `Cancel`. Confirming saves `active` and relaunches the app, which is outside what verify.sh tracks. <!-- src: src/features/hosts/ServersSettings.tsx:381 --> <!-- src: src/features/hosts/SwitchServerConfirm.tsx:76 --> <!-- src: src/features/hosts/SwitchServerConfirm.tsx:33 -->
- **Install (real host only).** On a row whose test returned `No server`, click `Install`. The row's status walks through `Checking the host`, `Downloading the server`, `Verifying the download`, `Copying to the host`, `Installing`, then the row is retested. <!-- src: src/features/hosts/ServersSettings.tsx:342 --> <!-- src: src/features/hosts/serverInstall.ts:6 --> <!-- src: src/features/hosts/useHostChecks.ts:89 -->
- **Proof.** Save screenshots of the validation alert, the unreachable row and the remove confirm, plus `list_hosts` JSON and `hosts.json` after the add, the edit and the remove.

## Gotchas

- `ul[aria-label="Servers"]` also exists in the status-bar server popover. Close that menu or scope to the settings content. <!-- src: src/features/hosts/ServerMenu.tsx:115 -->
- `Remove` is both the row icon's `aria-label` and the confirm button's text. Click the dialog's button by text, or scope it with `[role="dialog"]`.
- The default row's destination input is disabled. Changing it needs a switch first. <!-- src: src/features/hosts/ServersSettings.tsx:437 -->
- Probe results are kept only for this session. A reload shows the bogus row as `Not tested` until it's retested. <!-- src: src/lib/domain/hosts.ts:65 --> <!-- src: src/features/hosts/ServerSummary.tsx:16 -->
- A `Host *` ProxyCommand or a slow DNS in the user's ssh config can change the failure text or delay it. Read the row's `title` tooltip for ssh's own stderr. <!-- src: src/features/hosts/ServerSummary.tsx:57 -->
- An ssh password or host-key prompt opens the app's ssh prompt dialog (`list_ssh_prompts`). Answer `null` to cancel; never type a real password. <!-- src: src/lib/bindings.ts:353 -->
- `Update` on the connected row appears only when the server is outdated. That's the server-status recipe's territory.
