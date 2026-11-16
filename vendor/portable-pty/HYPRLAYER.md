`portable-pty` 0.9.0 exactly as published on crates.io, wired in by `[patch.crates-io]` in the workspace `Cargo.toml`.

One change, in `src/win/psuedocon.rs`: `CreatePseudoConsole` no longer passes `PSEUDOCONSOLE_INHERIT_CURSOR` (its now-unused constant is gone too).

Why: with that flag conhost sends `ESC[6n` and holds the child until a terminal answers, so a PTY no xterm.js has mounted (a hidden restored tab, an agent's spawn) hangs before its shell starts.

Drop this copy once upstream lets a caller choose the flags.
