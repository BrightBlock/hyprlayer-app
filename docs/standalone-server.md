# Running a standalone server

The runtime lives in a separate process, `hyprlayer-server` (ADR-0009). The
desktop ships its own copy and starts it for you, so nothing below is needed to
use the app on one machine. Install the package when you want a machine you
reach *over SSH* — a workstation you leave running, a build box — to host
sessions, terminals and scripts that outlive your laptop.

```sh
paru -S hyprlayer-server-bin                  # Arch
brew install brightblock/tap/hyprlayer-server # macOS, Linuxbrew
```

Neither starts anything. Run it in the foreground with `hyprlayer-server serve`,
or enable it for your user:

```sh
systemctl --user enable --now hyprlayer-server                       # Linux
cp .../ai.brightblock.hyprlayer.server.plist ~/Library/LaunchAgents/ # macOS
launchctl load ~/Library/LaunchAgents/ai.brightblock.hyprlayer.server.plist
```

A user unit, never a system one: every path the runtime uses is home-relative,
and on macOS the `claude` CLI it spawns reads its login from the user's
Keychain, which only a user session has unlocked. On Linux the unit survives a logout only with lingering
enabled (`loginctl enable-linger $USER`).

Point the desktop at it by adding the host to `<app config dir>/hosts.json`:

```json
{ "hosts": [{ "destination": "my-box", "label": "My box" }], "active": "my-box" }
```

`destination` is passed straight to `ssh`, so any alias your `~/.ssh/config`
resolves works. The app then runs `ssh <destination> hyprlayer-server relay`.
If ssh needs a password or asks you to trust a new host, the app asks in a
dialog and stores neither.

Four things to know before you rely on it:

- **There is no network listener and no authentication.** Authorization is
  filesystem permissions on the socket, plus SSH. Nothing is exposed to a
  network, and nothing else can be.
- **One server serves one profile and one user.** Every attached client shares
  one `settings.json`, one database and one session set, and can act on all of
  it. A second user means a second server on a second profile.
- **An app update ends every live session on that profile.** The protocol
  handshake is strict equality, so an upgraded desktop shuts the old server down
  and starts its own matching build. Quitting the app is free; updating is not.
- **`git` and `gh` on a server need that machine's own credentials.** The
  `SSH_AUTH_SOCK` your client sends is a path on the *client*, meaningless
  across machines — use agent forwarding (`ssh -A`) or a key on the server. And
  expose real binaries on the session's `PATH`: a CLI wrapped in an
  interactive-auth shim (1Password shell plugins, say) will *hang* a session
  rather than fail it, because there is no TTY to prompt on.

