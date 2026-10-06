use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
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

    fn min_version(self) -> Option<(&'static str, [u64; 3])> {
        match self {
            BinaryKind::Claude => Some(("2.1.0", [2, 1, 0])),
            BinaryKind::OpenCode => Some(("1.0.0", [1, 0, 0])),
            BinaryKind::Codex => Some(("0.155.1", [0, 155, 1])),
            _ => None,
        }
    }

    fn requires_version(self) -> bool {
        matches!(self, BinaryKind::Hyprlayer)
    }

    pub fn optional(self) -> bool {
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
    generation: AtomicU64,
    cwd: PathBuf,
    #[cfg_attr(not(unix), allow(dead_code))]
    shell: Option<OsString>,
}

impl BinaryResolver {
    pub fn initialize(
        environment: &crate::env::RuntimeEnvironment,
        overrides: HashMap<BinaryKind, PathBuf>,
    ) -> Self {
        let mut segments: Vec<PathBuf> = Vec::new();
        if let Some(process_path) = environment.var("PATH") {
            segments.extend(
                env::split_paths(process_path).filter(|p| !super::appimage::is_appdir_path(p)),
            );
        }
        segments.extend(well_known_dirs(environment));

        Self {
            augmented_path: RwLock::new(join_deduped(segments)),
            overrides: RwLock::new(overrides),
            cache: RwLock::new(HashMap::new()),
            generation: AtomicU64::new(0),
            cwd: environment.process_cwd.clone(),
            shell: environment.var("SHELL").map(OsString::from),
        }
    }

    pub async fn enrich_with_login_shell(&self) -> bool {
        #[cfg(unix)]
        {
            let Some(shell_path) = capture_login_shell_path(self.shell.as_deref()).await else {
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

    pub async fn augmented_path(&self) -> OsString {
        self.augmented_path.read().await.clone()
    }

    pub async fn resolve(&self, kind: BinaryKind) -> Option<PathBuf> {
        if let Some(p) = self.overrides.read().await.get(&kind) {
            return Some(p.clone());
        }
        let path = self.augmented_path.read().await.clone();
        which_in_augmented(kind.command_name(), &path, &self.cwd)
    }

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

    pub async fn status(&self, kind: BinaryKind) -> BinaryStatus {
        if let Some(cached) = self.cache.read().await.get(&kind) {
            return cached.clone();
        }
        let generation = self.generation.load(Ordering::SeqCst);
        let status = self.probe(kind).await;
        if generation == self.generation.load(Ordering::SeqCst) {
            self.cache.write().await.insert(kind, status.clone());
        }
        status
    }

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
        // `resolve` takes an override on trust, so a directory, a deleted file, or a
        // non-executable can reach here. Only `hyprlayer` requires a version, so for
        // every other kind an unrunnable path would otherwise probe to `None` and
        // fall through `state_for` to `Ok` — reported as "Found" with no version.
        if !is_runnable(&path) {
            return missing_status(kind, overridden);
        }

        let version = probe_version(&path).await;
        let state = state_for(kind, version.as_deref());
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

#[cfg(unix)]
fn is_runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_runnable(path: &Path) -> bool {
    path.is_file()
}

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

fn is_cargo_build_dir(dir: &Path) -> bool {
    dir.join(".fingerprint").is_dir() || dir.join(".cargo-lock").is_file()
}

// Exclude Cargo output without blacklisting install dirs such as /usr/bin.
fn build_artifact_dirs(exe: Option<&Path>) -> Vec<PathBuf> {
    let Some(dir) = exe.and_then(Path::parent) else {
        return Vec::new();
    };
    let root = match dir.file_name() {
        Some(name) if name == "deps" => dir.parent().unwrap_or(dir),
        _ => dir,
    };
    if !is_cargo_build_dir(root) {
        return Vec::new();
    }
    [root.to_path_buf(), root.join("deps")]
        .into_iter()
        .filter_map(|d| d.canonicalize().ok())
        .collect()
}

fn which_in_augmented(name: &str, augmented_path: &OsString, cwd: &Path) -> Option<PathBuf> {
    let forbidden = build_artifact_dirs(env::current_exe().ok().as_deref());
    let is_build_artifact = |p: &PathBuf| {
        if forbidden.is_empty() {
            return false;
        }
        p.canonicalize()
            .ok()
            .and_then(|c| c.parent().map(PathBuf::from))
            .is_some_and(|parent| forbidden.contains(&parent))
    };

    #[cfg(windows)]
    {
        if let Ok(mut hits) = which::which_in_all(format!("{name}.exe"), Some(augmented_path), cwd)
        {
            if let Some(exe) = hits.find(|p| !is_build_artifact(p)) {
                return Some(exe);
            }
        }
    }

    let found = which::which_in_all(name, Some(augmented_path), cwd)
        .ok()?
        .find(|p| !is_build_artifact(p));
    found
}

// XDG_DATA_HOME/APPDATA/LOCALAPPDATA are read from the supplied `RuntimeEnvironment`
// rather than via `dirs::data_dir()`/`dirs::data_local_dir()`, which would read the
// process environment ambiently; the fallbacks below mirror what those functions
// compute from the same variables.
fn well_known_dirs(environment: &crate::env::RuntimeEnvironment) -> Vec<PathBuf> {
    let home = &environment.home_dir;
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
    let platform = [
        environment
            .var("LOCALAPPDATA")
            .map(|local| PathBuf::from(local).join("Programs")),
        Some(home.join("scoop").join("shims")),
        environment.var("LOCALAPPDATA").map(|local| {
            PathBuf::from(local)
                .join("Microsoft")
                .join("WinGet")
                .join("Links")
        }),
        environment
            .var("APPDATA")
            .map(|appdata| PathBuf::from(appdata).join("npm")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let platform = [
        Some(PathBuf::from("/usr/local/bin")),
        Some(PathBuf::from("/var/lib/flatpak/exports/bin")),
        Some(PathBuf::from("/snap/bin")),
        Some(home.join(".nix-profile").join("bin")),
        Some(
            environment
                .var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"))
                .join("flatpak")
                .join("exports")
                .join("bin"),
        ),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    common.chain(platform).collect()
}

#[cfg(unix)]
async fn capture_login_shell_path(shell: Option<&std::ffi::OsStr>) -> Option<OsString> {
    let shell = shell
        .map(OsString::from)
        .unwrap_or_else(|| "/bin/zsh".into());
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
    line.split_whitespace()
        .find(|tok| parse_semver(tok).is_some())
        .map(|s| s.to_string())
}

pub(crate) fn parse_semver(s: &str) -> Option<[u64; 3]> {
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
    use crate::env::RuntimeEnvironment;

    fn real_environment() -> RuntimeEnvironment {
        RuntimeEnvironment {
            home_dir: dirs::home_dir().unwrap_or_default(),
            process_cwd: std::env::current_dir().unwrap_or_default(),
            process_env: std::env::vars_os().collect(),
            env_overlay: Default::default(),
        }
    }

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
        assert_eq!(state_for(BinaryKind::Hyprlayer, None), BinaryState::Missing);
        assert_eq!(
            state_for(BinaryKind::Hyprlayer, Some("1.5.6")),
            BinaryState::Ok
        );
        assert_eq!(state_for(BinaryKind::Git, None), BinaryState::Ok);
        assert_eq!(state_for(BinaryKind::Gh, None), BinaryState::Ok);
    }

    #[test]
    fn an_installed_app_forbids_nothing() {
        assert!(build_artifact_dirs(Some(Path::new("/usr/bin/Hyprlayer"))).is_empty());
        assert!(build_artifact_dirs(Some(Path::new("/usr/local/bin/Hyprlayer"))).is_empty());
        assert!(build_artifact_dirs(None).is_empty());
    }

    #[test]
    fn a_cargo_profile_dir_is_still_forbidden() {
        let tmp = tempfile::tempdir().unwrap();
        let profile = tmp.path().join("target").join("debug");
        std::fs::create_dir_all(profile.join(".fingerprint")).unwrap();
        std::fs::create_dir_all(profile.join("deps")).unwrap();

        for exe in [profile.join("Hyprlayer"), profile.join("deps/Hyprlayer")] {
            let forbidden = build_artifact_dirs(Some(&exe));
            assert!(
                forbidden.contains(&profile.canonicalize().unwrap()),
                "{exe:?} must forbid the profile dir"
            );
            assert!(
                forbidden.contains(&profile.join("deps").canonicalize().unwrap()),
                "{exe:?} must forbid the deps copy"
            );
        }
    }

    #[test]
    fn a_bin_dir_that_merely_looks_like_a_target_is_not_forbidden() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("target").join("release");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(build_artifact_dirs(Some(&dir.join("Hyprlayer"))).is_empty());
    }

    #[test]
    fn version_floor_still_flags_too_old() {
        assert_eq!(
            state_for(BinaryKind::Claude, Some("2.0.9")),
            BinaryState::TooOld {
                min: "2.1.0".into()
            }
        );
        assert_eq!(
            state_for(BinaryKind::Claude, Some("2.1.0")),
            BinaryState::Ok
        );
    }

    #[test]
    fn codex_version_floor_still_flags_too_old() {
        assert_eq!(
            state_for(BinaryKind::Codex, Some("0.155.0")),
            BinaryState::TooOld {
                min: "0.155.1".into()
            }
        );
        assert_eq!(
            state_for(BinaryKind::Codex, Some("0.155.1")),
            BinaryState::Ok
        );
    }

    #[tokio::test]
    async fn resolved_hyprlayer_without_version_reports_missing() {
        let resolver = BinaryResolver::initialize(&real_environment(), HashMap::new());
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

    // Every kind except `hyprlayer` returns `false` from `requires_version`, so before
    // `probe` checked the path these all reported `Ok` — "Found", with a blank version.
    #[tokio::test]
    async fn an_override_that_cannot_be_run_reports_missing_for_every_kind() {
        let dir = std::env::temp_dir();
        let absent = dir.join("hyprlayer-no-such-binary-9f3a");
        let not_executable = dir.join("hyprlayer-not-executable-9f3a");
        std::fs::write(&not_executable, b"#!/bin/sh\necho nope\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&not_executable, std::fs::Permissions::from_mode(0o644))
                .unwrap();
        }

        for kind in BinaryKind::ALL {
            for bogus in [dir.clone(), absent.clone(), not_executable.clone()] {
                let resolver = BinaryResolver::initialize(&real_environment(), HashMap::new());
                resolver.set_override(kind, Some(bogus.clone())).await;
                let status = resolver.status(kind).await;
                assert_eq!(
                    status.state,
                    BinaryState::Missing,
                    "{kind:?} with override {bogus:?} must not report healthy"
                );
                assert!(status.overridden, "{kind:?} keeps its manual-path flag");
                assert!(resolver.require(kind).await.is_err());
            }
        }

        let _ = std::fs::remove_file(&not_executable);
    }

    #[tokio::test]
    async fn resolves_git_on_dev_machines() {
        let resolver = BinaryResolver::initialize(&real_environment(), HashMap::new());
        let status = resolver.status(BinaryKind::Git).await;
        assert_eq!(
            status.state,
            BinaryState::Ok,
            "git should resolve: {status:?}"
        );
        #[cfg(not(windows))]
        assert!(status.version.is_some());
    }

    #[tokio::test]
    async fn an_override_beats_path() {
        let resolver = BinaryResolver::initialize(&real_environment(), HashMap::new());
        let fake = PathBuf::from("/nonexistent/fake-claude");
        resolver
            .set_override(BinaryKind::Claude, Some(fake.clone()))
            .await;
        assert_eq!(resolver.resolve(BinaryKind::Claude).await, Some(fake));
    }
}
