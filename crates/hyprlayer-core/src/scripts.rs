//! Pure helpers for the workstream lifecycle script runner (Setup/Run/Archive).
//!
//! The PTY spawn, live registry, and Tauri events live in the desktop shell
//! (`src-tauri`); this module holds the headless, testable bits: the script
//! kind, the fail-fast shell wrapper, free-port allocation, and dev-server port
//! detection.

use serde::{Deserialize, Serialize};

/// Which lifecycle script. Persisted as the `[scripts]` keys and carried on the
/// `ScriptRunChanged` event so the UI can route a run to the right tab/pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum ScriptKind {
    Setup,
    Run,
    Archive,
}

impl ScriptKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Run => "run",
            Self::Archive => "archive",
        }
    }
}

/// Env vars injected into every script run, on top of the user's environment.
/// `HL_PORT` is a free port allocated once per workstream for the Run server.
pub const ENV_PORT: &str = "HL_PORT";
pub const ENV_WORKTREE: &str = "HL_WORKTREE_PATH";
pub const ENV_BRANCH: &str = "HL_BRANCH";
pub const ENV_PROJECT: &str = "HL_PROJECT_PATH";
pub const ENV_SLUG: &str = "HL_SLUG";

/// Wrap a free-form command block so a non-zero line halts the sequence and the
/// shell's final exit status is the failing command's. POSIX shells get
/// `set -e`; PowerShell gets `$ErrorActionPreference='Stop'`. The returned block
/// is meant to be handed to the login shell via
/// `terminal::login_shell_args` (`sh -lc "<block>"` / `pwsh -Command "<block>"`).
pub fn build_block(commands: &str) -> String {
    #[cfg(not(windows))]
    {
        format!("set -e\n{commands}")
    }
    #[cfg(windows)]
    {
        format!("$ErrorActionPreference = 'Stop'\n{commands}")
    }
}

/// Allocate an ephemeral free TCP port on loopback. Best-effort: the port may be
/// taken again before the user's server binds it (TOCTOU), which is exactly why
/// the Run output is ALSO scanned for the actual port — see [`detect_port`].
pub fn free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

/// Scan one line of dev-server output for a localhost URL and return its port.
/// Recognizes `http(s)://localhost:PORT` and the same for `127.0.0.1`,
/// `0.0.0.0`, `[::1]`, and `[::]` (the all-interfaces form Python's
/// `http.server` and many Go/Node servers print), plus a bare `localhost:PORT`.
/// Returns the first hit.
///
/// Tolerant of surrounding ANSI color codes because the host:port substring
/// stays contiguous (Vite/Next color the whole URL as one unit).
pub fn detect_port(line: &str) -> Option<u16> {
    const HOSTS: [&str; 5] = ["localhost", "127.0.0.1", "0.0.0.0", "[::1]", "[::]"];
    for host in HOSTS {
        let needle = format!("{host}:");
        let mut from = 0;
        while let Some(rel) = line[from..].find(&needle) {
            let start = from + rel + needle.len();
            let digits: String = line[start..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(port) = digits.parse::<u16>() {
                if port != 0 {
                    return Some(port);
                }
            }
            from = start.max(from + 1);
            if from >= line.len() {
                break;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_block_prefixes_fail_fast() {
        let block = build_block("npm install\nnpm run build");
        #[cfg(not(windows))]
        assert!(block.starts_with("set -e\n"), "got: {block}");
        assert!(block.contains("npm run build"));
    }

    #[test]
    fn detect_port_finds_vite_local_url() {
        // The exact shape from the mockup's Run output.
        assert_eq!(
            detect_port("  ➜  Local:   http://localhost:5174/"),
            Some(5174)
        );
    }

    #[test]
    fn detect_port_ignores_non_url_port_mentions() {
        assert_eq!(detect_port("Port 5173 is in use, trying another one..."), None);
        assert_eq!(detect_port("  ➜  Network: use --host to expose"), None);
        assert_eq!(detect_port(""), None);
    }

    #[test]
    fn detect_port_handles_other_hosts() {
        assert_eq!(detect_port("ready on http://127.0.0.1:3000"), Some(3000));
        assert_eq!(detect_port("Listening on 0.0.0.0:8080"), Some(8080));
        assert_eq!(detect_port("serving http://[::1]:4321/"), Some(4321));
        // Python http.server / all-interfaces IPv6 form.
        assert_eq!(
            detect_port("Serving HTTP on :: port 62209 (http://[::]:62209/) ..."),
            Some(62209)
        );
    }

    #[test]
    fn detect_port_tolerates_ansi_wrapping() {
        // Color codes wrap the URL but the host:port stays contiguous.
        assert_eq!(
            detect_port("  \x1b[32m➜\x1b[39m  Local: \x1b[36mhttp://localhost:5174/\x1b[39m"),
            Some(5174)
        );
    }

    #[test]
    fn free_port_returns_a_usable_port() {
        let p = free_port().expect("allocate a port");
        assert!(p > 0);
    }

    /// The fail-fast preamble must actually halt a real shell on the first
    /// non-zero command (the "non-zero exit halts the sequence" contract).
    #[cfg(unix)]
    #[test]
    fn build_block_halts_the_shell_on_nonzero() {
        use std::process::Command;
        let block = build_block("echo one\nfalse\necho two");
        let out = Command::new("/bin/sh")
            .arg("-c")
            .arg(&block)
            .output()
            .expect("run sh");
        assert!(!out.status.success(), "block should fail on `false`");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("one"), "first command should run: {stdout:?}");
        assert!(!stdout.contains("two"), "must halt before `two`: {stdout:?}");
    }
}
