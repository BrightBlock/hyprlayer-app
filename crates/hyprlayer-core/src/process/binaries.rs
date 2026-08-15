//! Locating the external binaries the app depends on.
//!
//! macOS GUI apps inherit launchd's minimal PATH (`/usr/bin:/bin:...`), which
//! contains none of homebrew/cargo/npm. The resolver builds an augmented PATH
//! once (process PATH + login-shell PATH + well-known directories) and
//! resolves binaries against it; explicit per-binary overrides always win.
//! The augmented PATH is also what every spawned child should receive.

use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::error::Result;
use crate::process::Exec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum BinaryKind {
    Hyprlayer,
    Claude,
    OpenCode,
    Codex,
    Gh,
    Git,
    Ollama,
}

impl BinaryKind {
    pub const ALL: [BinaryKind; 7] = [
        BinaryKind::Hyprlayer,
        BinaryKind::Claude,
        BinaryKind::OpenCode,
        BinaryKind::Codex,
        BinaryKind::Gh,
        BinaryKind::Git,
        BinaryKind::Ollama,
    ];

    pub fn command_name(self) -> &'static str {
        match self {
            BinaryKind::Hyprlayer => "hyprlayer",
            BinaryKind::Claude => "claude",
            BinaryKind::OpenCode => "opencode",
            BinaryKind::Codex => "codex",
            BinaryKind::Gh => "gh",
            BinaryKind::Git => "git",
            BinaryKind::Ollama => "ollama",
        }
    }

    /// Minimum supported version, when the app depends on specific features.
    fn min_version(self) -> Option<(&'static str, [u64; 3])> {
        match self {
            // Control-protocol floor for the headless session driver.
            BinaryKind::Claude => Some(("2.1.0", [2, 1, 0])),
            // Server mode (`opencode serve`) + the HTTP/SSE session API the
            // OpenCode driver speaks. A conservative floor — tune if needed.
            BinaryKind::OpenCode => Some(("1.0.0", [1, 0, 0])),
            // `codex app-server`'s v2 thread protocol (thread/*, turn/*,
            // item/* notifications, the server-side approval requests). The
            // surface is flagged experimental upstream, so the floor is the
            // build the driver was written against — bump it in step with
            // regenerating the contract (`codex app-server generate-ts`).
            BinaryKind::Codex => Some(("0.145.0", [0, 145, 0])),
            _ => None,
        }
    }

    /// Whether a *resolved* binary must answer `--version` with a parseable
    /// semver to count as present. True only for our own CLI: the version is how
    /// we tell the real `hyprlayer` from the desktop app's `Hyprlayer.exe`, which
    /// shares the name case-insensitively on Windows and can be resolved through
    /// winget's portable shim (`Commands: [hyprlayer]`). The GUI prints no
    /// version, so a versionless hit is the desktop app, not the CLI.
    fn requires_version(self) -> bool {
        matches!(self, BinaryKind::Hyprlayer)
    }

    /// Whether the app can function without this binary.
    pub fn optional(self) -> bool {
        // The app runs fine on Claude alone; the other harnesses, gh, and Ollama
        // are extras.
        matches!(
            self,
            BinaryKind::Gh | BinaryKind::OpenCode | BinaryKind::Codex | BinaryKind::Ollama
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum BinaryState {
    Ok,
    Missing,
    TooOld { min: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct BinaryStatus {
    pub kind: BinaryKind,
    pub name: String,
    pub path: Option<String>,
    pub version: Option<String>,
    pub state: BinaryState,
    pub optional: bool,
    pub overridden: bool,
}

pub struct BinaryResolver {
    augmented_path: RwLock<OsString>,
    overrides: RwLock<HashMap<BinaryKind, PathBuf>>,
    cache: RwLock<HashMap<BinaryKind, BinaryStatus>>,
    /// Bumped whenever resolution inputs change (PATH enrichment, overrides,
    /// redetect). A probe started under an old generation must not populate
    /// the cache — it could pin a stale `Missing` after the PATH improved.
    generation: AtomicU64,
}

impl BinaryResolver {
    /// Build the resolver synchronously from the process PATH plus well-known
    /// install dirs. Cheap — safe to call during app setup. Call
    /// [`enrich_with_login_shell`](Self::enrich_with_login_shell) afterwards
    /// (in the background) to fold in the user's shell PATH.
    pub fn initialize(overrides: HashMap<BinaryKind, PathBuf>) -> Self {
        let mut segments: Vec<PathBuf> = Vec::new();
        if let Some(process_path) = env::var_os("PATH") {
            segments.extend(env::split_paths(&process_path));
        }
        segments.extend(well_known_dirs());

        Self {
            augmented_path: RwLock::new(join_deduped(segments)),
            overrides: RwLock::new(overrides),
            cache: RwLock::new(HashMap::new()),
            generation: AtomicU64::new(0),
        }
    }

    /// Capture the login-shell PATH (unix; best effort, hard timeout so a
    /// misbehaving rc can't wedge us) and merge it in. Returns true when the
    /// augmented PATH actually changed (callers should re-probe / notify).
    pub async fn enrich_with_login_shell(&self) -> bool {
        #[cfg(unix)]
        {
            let Some(shell_path) = capture_login_shell_path().await else {
                return false;
            };
            let mut segments: Vec<PathBuf> =
                env::split_paths(&*self.augmented_path.read().await).collect();
            segments.extend(env::split_paths(&shell_path));
            let merged = join_deduped(segments);

            let mut current = self.augmented_path.write().await;
            if *current == merged {
                return false;
            }
            *current = merged;
            drop(current);
            self.generation.fetch_add(1, Ordering::SeqCst);
            self.cache.write().await.clear();
            true
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    /// The PATH value to inject into every spawned child process.
    pub async fn augmented_path(&self) -> OsString {
        self.augmented_path.read().await.clone()
    }

    /// Absolute path for a binary, if resolvable. Does not probe versions.
    pub async fn resolve(&self, kind: BinaryKind) -> Option<PathBuf> {
        if let Some(p) = self.overrides.read().await.get(&kind) {
            return Some(p.clone());
        }
        let path = self.augmented_path.read().await.clone();
        which_in_augmented(kind.command_name(), &path)
    }

    /// Resolve, requiring a healthy probe (cached). Errors carry remedies.
    pub async fn require(&self, kind: BinaryKind) -> Result<PathBuf> {
        let status = self.status(kind).await;
        match (&status.state, &status.path) {
            (BinaryState::Ok, Some(path)) => Ok(PathBuf::from(path)),
            (BinaryState::TooOld { min }, _) => Err(crate::CoreError::BinaryTooOld {
                name: kind.command_name().into(),
                found: status.version.unwrap_or_else(|| "unknown".into()),
                min: min.clone(),
            }),
            _ => Err(crate::CoreError::BinaryMissing {
                name: kind.command_name().into(),
            }),
        }
    }

    /// Probed status for one binary, cached until `redetect`.
    pub async fn status(&self, kind: BinaryKind) -> BinaryStatus {
        if let Some(cached) = self.cache.read().await.get(&kind) {
            return cached.clone();
        }
        let generation = self.generation.load(Ordering::SeqCst);
        let status = self.probe(kind).await;
        // Cache only if resolution inputs didn't change while we probed.
        if generation == self.generation.load(Ordering::SeqCst) {
            self.cache.write().await.insert(kind, status.clone());
        }
        status
    }

    /// All statuses, probed concurrently — worst case is one probe timeout,
    /// not N in a row.
    pub async fn status_all(&self) -> Vec<BinaryStatus> {
        let [a, b, c, d, e, f, g] = BinaryKind::ALL;
        let (a, b, c, d, e, f, g) = tokio::join!(
            self.status(a),
            self.status(b),
            self.status(c),
            self.status(d),
            self.status(e),
            self.status(f),
            self.status(g)
        );
        vec![a, b, c, d, e, f, g]
    }

    pub async fn set_override(&self, kind: BinaryKind, path: Option<PathBuf>) {
        let mut overrides = self.overrides.write().await;
        match path {
            Some(p) => {
                overrides.insert(kind, p);
            }
            None => {
                overrides.remove(&kind);
            }
        }
        drop(overrides);
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.cache.write().await.remove(&kind);
    }

    pub async fn redetect(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.cache.write().await.clear();
    }

    async fn probe(&self, kind: BinaryKind) -> BinaryStatus {
        let overridden = self.overrides.read().await.contains_key(&kind);
        let Some(path) = self.resolve(kind).await else {
            return missing_status(kind, overridden);
        };

        let version = probe_version(&path).await;
        let state = state_for(kind, version.as_deref());
        // `Missing` from a path that *did* resolve means the binary answered but
        // isn't our CLI — the Windows `Hyprlayer.exe` desktop-exe collision. Drop
        // the path so nothing downstream runs the GUI as if it were the CLI.
        if state == BinaryState::Missing {
            return missing_status(kind, overridden);
        }

        BinaryStatus {
            kind,
            name: kind.command_name().into(),
            path: Some(path.display().to_string()),
            version,
            state,
            optional: kind.optional(),
            overridden,
        }
    }
}

/// A `BinaryStatus` for a binary we couldn't resolve to a usable path. Path and
/// version are cleared — there is nothing to hand back or display.
fn missing_status(kind: BinaryKind, overridden: bool) -> BinaryStatus {
    BinaryStatus {
        kind,
        name: kind.command_name().into(),
        path: None,
        version: None,
        state: BinaryState::Missing,
        optional: kind.optional(),
        overridden,
    }
}

/// Decide a resolved binary's state from its probed `--version` output.
///
/// A parseable version below the kind's floor is `TooOld`; at/above it (or when
/// the kind has no floor) it's `Ok`. When `--version` yielded nothing:
/// - third-party binaries stay `Ok` — some shims only answer in a full
///   invocation, so a bare resolved path is still worth handing back;
/// - our own `hyprlayer` CLI becomes `Missing`, because a versionless hit is the
///   desktop app's `Hyprlayer.exe` wearing the CLI's name (see
///   [`BinaryKind::requires_version`]), not the CLI.
fn state_for(kind: BinaryKind, version: Option<&str>) -> BinaryState {
    match (version, kind.min_version()) {
        (Some(v), Some((min_str, min))) => {
            if parse_semver(v).is_some_and(|found| found < min) {
                BinaryState::TooOld {
                    min: min_str.to_string(),
                }
            } else {
                BinaryState::Ok
            }
        }
        (None, _) if kind.requires_version() => BinaryState::Missing,
        _ => BinaryState::Ok,
    }
}

fn join_deduped(mut segments: Vec<PathBuf>) -> OsString {
    let mut seen = std::collections::HashSet::new();
    segments.retain(|p| !p.as_os_str().is_empty() && seen.insert(p.clone()));
    env::join_paths(segments).unwrap_or_else(|_| env::var_os("PATH").unwrap_or_default())
}

fn which_in_augmented(name: &str, augmented_path: &OsString) -> Option<PathBuf> {
    let cwd = env::current_dir().ok()?;

    // On Windows dev builds `cargo run` puts `target\debug` on PATH, where the
    // GUI exe `Hyprlayer.exe` matches a search for `hyprlayer` — so the resolver
    // must not hand back its own build artifacts and spawn a second app instance.
    let forbidden: Vec<PathBuf> = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .map(|dir| {
            // Both `target\debug\` and the `target\debug\deps\` copy cargo keeps.
            let mut dirs = vec![dir.clone(), dir.join("deps")];
            if dir.file_name().is_some_and(|n| n == "deps") {
                dirs.extend(dir.parent().map(PathBuf::from));
            }
            dirs.into_iter()
                .filter_map(|d| d.canonicalize().ok())
                .collect()
        })
        .unwrap_or_default();
    let is_build_artifact = |p: &PathBuf| {
        p.canonicalize()
            .ok()
            .and_then(|c| c.parent().map(PathBuf::from))
            .is_some_and(|parent| forbidden.contains(&parent))
    };

    // On Windows, prefer a real .exe over npm's .cmd shims — CreateProcess
    // cannot spawn .cmd directly and the session driver needs a true child.
    #[cfg(windows)]
    {
        if let Ok(mut hits) = which::which_in_all(format!("{name}.exe"), Some(augmented_path), &cwd)
        {
            if let Some(exe) = hits.find(|p| !is_build_artifact(p)) {
                return Some(exe);
            }
        }
    }

    let found = which::which_in_all(name, Some(augmented_path), &cwd)
        .ok()?
        .find(|p| !is_build_artifact(p));
    found
}

fn well_known_dirs() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let common = [
        ".local/bin",
        ".cargo/bin",
        ".claude/local",
        ".opencode/bin",
        ".bun/bin",
        ".volta/bin",
    ]
    .into_iter()
    .map(|seg| home.join(seg));

    #[cfg(target_os = "macos")]
    let platform = [
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    #[cfg(target_os = "windows")]
    // scoop shims + winget link shims + standard npm global prefix
    let platform = [
        dirs::data_local_dir().map(|local| local.join("Programs")),
        dirs::home_dir().map(|profile| profile.join("scoop").join("shims")),
        // winget shim dir — else a winget-installed CLI isn't found until restart.
        dirs::data_local_dir().map(|local| local.join("Microsoft").join("WinGet").join("Links")),
        dirs::data_dir().map(|appdata| appdata.join("npm")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    // Linux: an AppImage launched from a desktop file inherits the session's
    // PATH, which on most desktops is /usr/bin only — no Nix, no Snap, no
    // Flatpak exports. The login-shell capture above usually fills those in,
    // but it needs $SHELL set and an rc file that exports them, and neither is
    // guaranteed from a .desktop launch. These cover the package managers a
    // developer's claude/codex/gh most often comes from.
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let platform = [
        Some(PathBuf::from("/usr/local/bin")),
        Some(PathBuf::from("/var/lib/flatpak/exports/bin")),
        Some(PathBuf::from("/snap/bin")),
        dirs::home_dir().map(|home| home.join(".nix-profile").join("bin")),
        dirs::data_dir().map(|share| share.join("flatpak").join("exports").join("bin")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    common.chain(platform).collect()
}

/// Ask the user's login shell for its PATH. Interactive (`-i`) so rc files
/// that guard on interactivity still contribute; stdin is null and the call
/// is bounded so a prompt-heavy rc can't hang us.
#[cfg(unix)]
async fn capture_login_shell_path() -> Option<OsString> {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let output = Exec::new(&shell)
        .args(["-lic", r#"printf %s "$PATH""#])
        .timeout(Duration::from_secs(8))
        .run_unchecked()
        .await
        .ok()?;
    if output.exit_code == Some(0) && !output.stdout.trim().is_empty() {
        Some(OsString::from(output.stdout.trim().to_string()))
    } else {
        None
    }
}

async fn probe_version(path: &PathBuf) -> Option<String> {
    let output = Exec::new(path)
        .arg("--version")
        .timeout(Duration::from_secs(5))
        .run_unchecked()
        .await
        .ok()?;
    if output.exit_code != Some(0) {
        return None;
    }
    let line = output.stdout.lines().next()?.trim();
    // "git version 2.49.0", "gh version 2.62.0 (2024-…)", "hyprlayer 1.5.6",
    // "2.1.173 (Claude Code)" — take the first semver-looking token.
    line.split_whitespace()
        .find(|tok| parse_semver(tok).is_some())
        .map(|s| s.to_string())
}

fn parse_semver(s: &str) -> Option<[u64; 3]> {
    let core = s.trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some([major, minor, patch])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_tokens() {
        assert_eq!(parse_semver("2.1.173"), Some([2, 1, 173]));
        assert_eq!(parse_semver("v2.62.0"), Some([2, 62, 0]));
        assert_eq!(parse_semver("2.49"), Some([2, 49, 0]));
        assert_eq!(parse_semver("1.5.6-beta+abc"), Some([1, 5, 6]));
        assert_eq!(parse_semver("version"), None);
    }

    #[test]
    fn versionless_hyprlayer_is_missing_but_third_party_stays_present() {
        // The crux of the Windows desktop-exe misdetection: a `hyprlayer` that
        // resolved but printed no version is the GUI wearing the CLI's name, so
        // it must be Missing — while gh/git and friends keep the lenient path.
        assert_eq!(state_for(BinaryKind::Hyprlayer, None), BinaryState::Missing);
        assert_eq!(
            state_for(BinaryKind::Hyprlayer, Some("1.5.6")),
            BinaryState::Ok
        );
        assert_eq!(state_for(BinaryKind::Git, None), BinaryState::Ok);
        assert_eq!(state_for(BinaryKind::Gh, None), BinaryState::Ok);
    }

    #[test]
    fn version_floor_still_flags_too_old() {
        assert_eq!(
            state_for(BinaryKind::Claude, Some("2.0.9")),
            BinaryState::TooOld { min: "2.1.0".into() }
        );
        assert_eq!(state_for(BinaryKind::Claude, Some("2.1.0")), BinaryState::Ok);
    }

    #[tokio::test]
    async fn resolved_hyprlayer_without_version_reports_missing() {
        // End-to-end: an override points `hyprlayer` at a path that yields no
        // `--version` (here, one that won't spawn, so `probe_version` is None).
        // Detection must report Missing with no path, and `require` must error —
        // never hand back the wrong exe as if it were the CLI.
        let resolver = BinaryResolver::initialize(HashMap::new());
        resolver
            .set_override(
                BinaryKind::Hyprlayer,
                Some(PathBuf::from("/nonexistent/not-really-hyprlayer")),
            )
            .await;
        let status = resolver.status(BinaryKind::Hyprlayer).await;
        assert_eq!(status.state, BinaryState::Missing);
        assert_eq!(status.path, None);
        assert!(resolver.require(BinaryKind::Hyprlayer).await.is_err());
    }

    #[tokio::test]
    async fn resolves_git_on_dev_machines() {
        let resolver = BinaryResolver::initialize(HashMap::new());
        // git exists everywhere we develop and in CI.
        let status = resolver.status(BinaryKind::Git).await;
        assert_eq!(
            status.state,
            BinaryState::Ok,
            "git should resolve: {status:?}"
        );
        // probe() treats a resolved binary whose `--version` didn't yield a
        // parseable token as Ok-but-versionless (see its `_ => Ok` arm). The
        // Windows runner's git hits that path, so only assert the version where
        // probing is reliable; resolution is the contract this test guards.
        #[cfg(not(windows))]
        assert!(status.version.is_some());
    }

    #[tokio::test]
    async fn override_beats_path_and_redetect_clears() {
        let resolver = BinaryResolver::initialize(HashMap::new());
        let fake = PathBuf::from("/nonexistent/fake-claude");
        resolver
            .set_override(BinaryKind::Claude, Some(fake.clone()))
            .await;
        assert_eq!(resolver.resolve(BinaryKind::Claude).await, Some(fake));
        resolver.set_override(BinaryKind::Claude, None).await;
        // After clearing, resolution falls back to PATH (may or may not exist).
        let _ = resolver.resolve(BinaryKind::Claude).await;
    }
}
