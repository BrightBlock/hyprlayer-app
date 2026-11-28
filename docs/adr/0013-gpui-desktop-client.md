# ADR 0013: The desktop client is GPUI

**Status:** proposed · 2026-11-28 (G4 of `thoughts/shared/plans/2026-11-27-gpui-desktop-client.md`; accepted at G14)

## Decision

The desktop becomes one native Rust binary on GPUI. It links `hyprlayer-client` and talks to
`hyprlayer-server` over the same socket or SSH relay as the Tauri app. The runtime stays in
the server (ADR-0009). Until the cutover the binary is `hyprlayer-native`, a dev build only.

### Library and pins

- `gpui-component =0.11.0` is the component library. It is built against exactly
  `gpui-pre =0.3.14`, and a window also needs `gpui-pre-platform =0.3.14`. The three are pinned
  together with `=`, in `hyprlayer-ui` and `hyprlayer-desktop`, and move together in one PR.
- `cargo tree -d | grep -i gpui` shows one `gpui-pre`. A second `gpui` in the tree is the
  September spike's failure and blocks the PR that adds it.
- Zed's GPL crates (`editor`, `terminal_view`, `markdown`, `ui`) are reference reading only.
- The fallbacks, if a pin ever fails to build, are upstream Zed GPUI, `gpui-unofficial` and
  `gpui-ce`, in that order.

### Crate layering

```text
hyprlayer-client   connection, supervisor, hosts, ssh, identity, geometry (no UI)
       ▼
hyprlayer-ui       theme tokens, type scale, row tiers, Dialog, Popover, icons, the tracker
       ▼
hyprlayer-desktop  connection entity, Query<K, T>, views, shell, drive bridge, hyprlayer-native
```

Cargo enforces the direction: `hyprlayer-ui` cannot name `hyprlayer-desktop`. The server
crates (`hyprlayer-{models,core,server,proto}`) gain no GPUI dependency. ADR-0006's semantic
rules (row tiers, hover, `Dialog` sizes, type tokens, icon keys) become `hyprlayer-ui` types.
Its CSS-only rules do not apply.

Server state goes through one generic `Query<K, T>` entity per read. `events::topics_for` maps
each `RuntimeEvent` to the topics it invalidates. A reconnect and an `EventsMissed` re-read
every query. A query's value counts as current only if it was read on the live connection, so a
lost link never presents stale rows.

### Paths and profiles

- The client resolves the same per-identifier dirs Tauri does (`hyprlayer_client::paths`). The
  default identifier is the Tauri dev app's, `ai.brightblock.hyprlayer.dev`, so both dev
  clients share one profile, one server and one `hosts.json`.
- `--identifier`, `--profile-dir <dir>` (config, data and logs under `<dir>`) and
  `--attach <server-data-dir>` choose the profile. `--attach` connects to a running server and
  never spawns or replaces one.
- The client ignores `HYPRLAYER_APP_{CONFIG,DATA,LOG}_DIR`. A shell may export the release
  profile's dirs, and the supervisor evicts a mismatched server on its own profile
  (`replaces_own_build`).
- Its own files never collide with the Tauri app's: `hyprlayer-native.log` (5 MB, five kept),
  `native-window.json`, `native-instance.lock` and the `native.sock` single-instance socket,
  which sits beside `drive.sock`.
- The window class is `Hyprlayer` for the release identifier, `Hyprlayer-Dev` for `.dev`, and
  `Hyprlayer-Debug` for any other identifier, so a verify profile's window matches the verify
  skill's Hyprland rule and never the user's.

### Coexistence

Both clients may run at once on one profile (the server already serves several desktops).
Both read the one `MIN_SERVER_VERSION`, so neither evicts the other's server for being too old.
`hosts.json` writes merge. Client-local state starts from defaults in the GPUI client and never
touches the webview's storage. New user-facing features land only in the GPUI client once G7
merges.

### Drive bridge

Debug builds on Unix serve newline JSON on `hl-native-<profile hash>/drive.sock` under
`$TMPDIR` (macOS) or `$XDG_RUNTIME_DIR/hyprlayer` (Linux), short enough for `sun_path`. It is
not the server's `socket::runtime_dir`, because a server starting on the same profile deletes
every socket there. The path is logged and written to `<data dir>/native-drive-socket`. Every
answer is one line with `"ok"`.

| Op | Does |
|---|---|
| `ids` | every tracked element painted in the last frame, with window bounds |
| `find {id}` | one element's bounds |
| `click {id}` | mouse move, down and up at its centre, dispatched in-app |
| `key {keystroke}` | an in-app keystroke (`cmd-q`, `ctrl-c`) |
| `key {keystroke, mode: "compositor", focus?}` | the key through Hyprland's `hl.dsp.send_shortcut`, focusing the window first if asked |
| `state` | `Desktop::dump` (connection state, lists, fetch counts, painted ids) and whether the window is `visible` |
| `capture {path}` | `screencapture -l <window>` on macOS, `grim -g` under Hyprland |
| `notify {title, body}` | one notification through the client's notifier |
| `quit` | quits after answering |

Tracked ids are cleared at the top of each root render and recorded in prepaint, clipped to the
content mask, so `ids` lists what the last frame painted. On Wayland a synthetic in-app key has
no input serial and cannot set the clipboard, so copy checks use the compositor mode.

### Quitting

`cx.quit()` runs quit observers but no destructors, so `kill_on_drop` never fires. The quit
observer saves the window bounds and calls `supervisor::stop_relays()`, which ends each ssh
relay by pid. The local server is left running as the detached orphan ADR-0009 makes it, shared
with every other client of the profile. Closing the window quits.

### Notifications

One blocking `notify::send`: `UNUserNotificationCenter` on macOS, `notify-rust` elsewhere. On
macOS the client first checks for a bundle identifier, because the center raises in an
unbundled binary. Its delegate presents banners while the app is frontmost, which the system
otherwise silences.

### The release gate

`scripts/release.sh` runs nextest with `--exclude hyprlayer-ui --exclude hyprlayer-desktop`,
so a broken GPUI crate cannot fail a server release. The GPUI E2E tier is
`cargo test -p hyprlayer-e2e --features gpui --test gpui_skeleton`. The `gpui` feature makes
`hyprlayer-desktop` an optional dependency of the E2E crate, so no default build pulls GPUI in.
`scripts/check-server-compat.mjs` reads both GPUI crates, so their RPC calls meet
`MIN_SERVER_VERSION`.

## Rationale

The plan's motivation is WebKitGTK performance on Linux. GPUI with `gpui-component` gives
native widgets on all three platforms, and the pinned pair from crates.io compiled as one
`gpui` in G1 where the September spike's mix failed. Keeping the client a thin layer over
`hyprlayer-client` means the compiler checks the RPC contract against the server crate it links.

## Consequences

- Adding the GPUI crates grew `Cargo.lock` and patch-bumped crates the server shares. Review
  such bumps in the PR that causes them.
- `@fontsource` ships only woff and woff2, which neither Core Text nor fontdb loads. The client
  registers no fonts yet, and an uninstalled Inter or JetBrains Mono falls back to the system's.
- A window AppKit reports occluded (covered, minimised, hidden or on another Space) stops its
  display link and draws nothing, so `painted` keeps the last visible frame. `state` reports
  `visible` so a check can tell.

## Open items

- **Windows build path.** Release builds cannot cross-compile, because `gpui-pre-windows`
  compiles its shaders with `fxc.exe` on a Windows host only. Either build natively on
  `nexus-win`, or vendor `gpui-pre-windows` with committed `shaders_bytes.rs` and keep the
  cargo-xwin container. Decide before the first Windows live check. G4 targets macOS and Linux.
- Omarchy theme polling and the unsupported-GPU screen are not built yet.
- `hosts.json` is read once at launch. A host switch made in the Tauri client needs a relaunch,
  and the client cannot answer ssh prompts yet (batch only).
- The Tauri app's quit confirmation (`settings.quit.confirm`) is not built yet.
- The macOS notification path, delegate included, is unproven until a live check posts one from
  a bundled `.app`.
- The fonts above.
