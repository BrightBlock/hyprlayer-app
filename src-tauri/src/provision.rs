//! Puts a version-keyed `hyprlayer-server` on a remote host and runs one of
//! those (plan Phase 7), the way VS Code Remote-SSH materializes a pinned
//! server: versions install side by side, nothing upgrades in place, and the
//! client only ever invokes an absolute path under `~/.hyprlayer/server`. A
//! packaged server on the host keeps `PATH` for headless use; the two never
//! collide.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::hosts::Host;
use crate::server_version::{self, SERVER_VERSION};
use crate::ssh::Ssh;
use crate::supervisor;

const RELEASE_BASE: &str = "https://github.com/BrightBlock/hyprlayer-releases/releases/download";
const SUMS_ASSET: &str = "hyprlayer-server-sha256sums.txt";
const CONNECT_TIMEOUT: &str = "20";
const DOWNLOAD_TIMEOUT: &str = "600";

/// Where a local binary for another platform came from. The UI reports one
/// fewer step for `Cache`/`Beside` — there is nothing to download or verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum BinarySource {
    Cache,
    Beside,
    Download,
}

/// Whether the host already holds the binary this client is about to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum HostServerState {
    /// Same path, and the same bytes wherever we could compare them.
    Ready,
    /// Nothing at the version-keyed path.
    Missing,
    /// Present at the version-keyed path, different bytes. An unreleased build
    /// and the release it descends from share a version, so they share a
    /// directory name (plan Phase 7 §3.2).
    Stale,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HostProvisioning {
    pub version: String,
    pub triple: String,
    pub state: HostServerState,
    /// Every version directory under `~/.hyprlayer/server`. A non-empty list is
    /// what separates "offer the install" from "a version bump on a host the
    /// user already accepted" (plan Phase 7 §4) — derived from the host's own
    /// state rather than a flag this side would have to keep in step with it.
    pub installed_versions: Vec<String>,
    /// What the relay runs: this client's version when `Ready`, else the newest
    /// installed version at or above the minimum. `None` means install first.
    pub usable_version: Option<String>,
    pub source: BinarySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ProvisionStep {
    Probe,
    Download,
    Verify,
    Upload,
    Install,
    Done,
}

pub struct LocalBinary {
    pub path: PathBuf,
    pub source: BinarySource,
}

fn asset_name(triple: &str) -> String {
    if triple.contains("windows") {
        format!("hyprlayer-server-{triple}.exe")
    } else {
        format!("hyprlayer-server-{triple}")
    }
}

/// The staging script's destination and the downloader's are deliberately the
/// same path: pre-staging a binary *is* seeding the download cache, so there
/// is one lookup rather than a dev branch and a real branch.
pub fn cache_path(app_data_dir: &Path, triple: &str) -> PathBuf {
    app_data_dir
        .join("server-cache")
        .join(SERVER_VERSION)
        .join(asset_name(triple))
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

/// Cache, then beside the app binary, then a download (plan Phase 7 §3.3).
/// Nothing puts an other-platform binary beside the app today — they are
/// deliberately not bundled — so that arm costs one `exists()` and means a
/// later decision to bundle needs no code change.
pub fn resolve_local(
    app_data_dir: &Path,
    exe_dir: Option<&Path>,
    triple: &str,
) -> Option<LocalBinary> {
    let cached = cache_path(app_data_dir, triple);
    if cached.is_file() {
        return Some(LocalBinary {
            path: cached,
            source: BinarySource::Cache,
        });
    }
    let beside = exe_dir?.join(asset_name(triple));
    beside.is_file().then_some(LocalBinary {
        path: beside,
        source: BinarySource::Beside,
    })
}

/// Quoted so a home directory with a space still resolves, and expanded by the
/// remote shell rather than by us — the client never guesses the host's home.
fn remote_binary(version: &str) -> String {
    format!("\"$HOME/.hyprlayer/server/{version}/hyprlayer-server\"")
}

/// What `ssh <host> …` runs: an absolute, version-keyed path under
/// `~/.hyprlayer/server`, never whatever `hyprlayer-server` resolves to on the
/// host's PATH.
pub fn relay_command(version: &str) -> String {
    format!("{} relay", remote_binary(version))
}

/// Prints the tails of the host's server logs; works with no server running.
pub fn logs_command(version: &str) -> String {
    format!("{} logs", remote_binary(version))
}

/// Stops the host's running server, whichever version it is, and waits for it
/// to go, so the next relay starts `version` rather than reattaching to it.
/// Nothing running is success; a server still answering after 10s is not.
fn stop_script(version: &str) -> String {
    let bin = remote_binary(version);
    format!(
        "{bin} status >/dev/null 2>&1 || exit 0; {bin} stop >&2; i=0; \
         while {bin} status >/dev/null 2>&1; do \
         [ $i -ge 50 ] && {{ echo 'the server did not stop within 10s' >&2; exit 1; }}; \
         sleep 0.2; i=$((i+1)); done; exit 0"
    )
}

/// Installs this client's server if the host lacks it, then stops whatever is
/// running there. Ends every session and terminal on that host.
pub async fn update(
    host: &Host,
    app_data_dir: &Path,
    on_step: impl Fn(ProvisionStep),
    ssh: &Ssh,
) -> Result<(), AppError> {
    if readiness(host, app_data_dir, ssh).await?.state != HostServerState::Ready {
        provision(host, app_data_dir, &on_step, ssh).await?;
    }
    log::warn!(
        "stopping the server on {} to start {SERVER_VERSION}; its sessions and terminals end",
        host.destination
    );
    supervisor::ssh_capture(host, &stop_script(SERVER_VERSION), ssh).await?;
    on_step(ProvisionStep::Done);
    Ok(())
}

/// Receives the pushed binary into `<version>.tmp/`. An interrupted transfer
/// leaves a directory no lookup ever consults and the next run wipes.
fn stage_script(version: &str) -> String {
    format!(
        "set -e; d=\"$HOME/.hyprlayer/server/{version}.tmp\"; \
         rm -rf \"$d\"; mkdir -p \"$d\"; cat > \"$d/hyprlayer-server\""
    )
}

/// The only step that makes a version directory exist, and it runs in a second
/// round trip so a truncated push cannot reach it. The size check is what
/// catches a transfer that ended early but exited clean.
fn commit_script(version: &str, bytes: u64) -> String {
    format!(
        "set -e; r=\"$HOME/.hyprlayer/server\"; d=\"$r/{version}.tmp\"; \
         n=\"$(wc -c < \"$d/hyprlayer-server\")\"; \
         [ \"$n\" -eq {bytes} ] || {{ echo \"received $n of {bytes} bytes\" >&2; exit 1; }}; \
         chmod 755 \"$d/hyprlayer-server\"; \
         rm -rf \"$r/{version}\"; mv \"$d\" \"$r/{version}\""
    )
}

/// One round trip answering everything the decision needs: what the host is,
/// what this version's binary hashes to there, and what else this client family
/// has installed. Always exits 0 — an absent directory is an answer, not a
/// failure. `sha256sum` on Linux, `shasum` on macOS; a host with neither emits
/// no `@sha` and is trusted on its version. Versions are listed with `ls`, not a
/// glob: zsh, the macOS login shell, fails an unmatched glob.
fn inspect_script(version: &str) -> String {
    format!(
        "uname -sm; r=\"$HOME/.hyprlayer/server\"; b=\"$r/{version}/hyprlayer-server\"; \
         if [ -x \"$b\" ]; then echo '@installed'; \
         echo \"@sha $({{ sha256sum \"$b\" || shasum -a 256 \"$b\"; }} 2>/dev/null \
         | awk '{{print $1}}')\"; fi; \
         ls \"$r\" 2>/dev/null | while read -r d; do [ -d \"$r/$d\" ] && echo \"@version $d\"; done; \
         exit 0"
    )
}

struct Inspected {
    uname: String,
    installed: bool,
    sha: Option<String>,
    versions: Vec<String>,
}

fn parse_inspect(stdout: &str) -> Inspected {
    let mut uname = String::new();
    let mut installed = false;
    let mut sha = None;
    let mut versions = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line == "@installed" {
            installed = true;
        } else if let Some(hex) = line.strip_prefix("@sha ") {
            sha = Some(hex.trim())
                .filter(|h| !h.is_empty())
                .map(str::to_string);
        } else if let Some(name) = line.strip_prefix("@version ") {
            // A staging directory is not an install; it is what an interrupted
            // push leaves behind.
            if !name.ends_with(".tmp") {
                versions.push(name.to_string());
            }
        } else if uname.is_empty() && !line.is_empty() {
            uname = line.to_string();
        }
    }
    versions.sort();
    Inspected {
        uname,
        installed,
        sha,
        versions,
    }
}

/// The version alone cannot answer this: `CARGO_PKG_VERSION` is the same string
/// for a release and for every unreleased build after it, so a bare existence
/// test accepts whichever landed first (plan Phase 7 §3.2).
///
/// `local_sha` is `None` when the only way to get local bytes would be a
/// download, and then the version is trusted rather than fetched to compare.
/// The asymmetry is the shape of the problem, not a compromise: a build whose
/// version lies is one that was staged, so its bytes are already in hand, while
/// a released version resolves to an immutable asset at that tag.
fn state_for(installed: bool, host_sha: Option<&str>, local_sha: Option<&str>) -> HostServerState {
    if !installed {
        return HostServerState::Missing;
    }
    match (host_sha, local_sha) {
        (Some(host), Some(local)) if !host.eq_ignore_ascii_case(local) => HostServerState::Stale,
        _ => HostServerState::Ready,
    }
}

/// A stale install of this client's own version is a dev build shadowing the
/// release, so it blocks rather than falling back to an older version.
fn usable_version(state: HostServerState, installed: &[String]) -> Option<String> {
    match state {
        HostServerState::Ready => Some(SERVER_VERSION.to_string()),
        HostServerState::Stale => None,
        HostServerState::Missing => {
            server_version::newest_usable(installed.iter().map(String::as_str)).map(str::to_string)
        }
    }
}

pub(crate) fn sha256_file(path: &Path) -> Option<String> {
    Some(format!("{:x}", Sha256::digest(std::fs::read(path).ok()?)))
}

/// What the connect path checks before it relays, and what the install offer
/// reads. Never downloads.
pub async fn readiness(
    host: &Host,
    app_data_dir: &Path,
    ssh: &Ssh,
) -> Result<HostProvisioning, AppError> {
    Ok(inspect(host, app_data_dir, ssh).await?.0)
}

/// `readiness`, plus the hash of the binary the relay will run — what a
/// server that relay attaches to should report as its build.
pub async fn inspect(
    host: &Host,
    app_data_dir: &Path,
    ssh: &Ssh,
) -> Result<(HostProvisioning, Option<String>), AppError> {
    let stdout = supervisor::ssh_capture(host, &inspect_script(SERVER_VERSION), ssh).await?;
    let seen = parse_inspect(&stdout);
    let triple = supervisor::triple_for_uname(host, &seen.uname)?;
    let local = resolve_local(app_data_dir, exe_dir().as_deref(), triple);
    let local_sha = local
        .as_ref()
        .filter(|_| seen.installed)
        .and_then(|found| sha256_file(&found.path));
    let state = state_for(seen.installed, seen.sha.as_deref(), local_sha.as_deref());
    let provisioning = HostProvisioning {
        version: SERVER_VERSION.to_string(),
        triple: triple.to_string(),
        state,
        usable_version: usable_version(state, &seen.versions),
        installed_versions: seen.versions,
        source: local
            .map(|found| found.source)
            .unwrap_or(BinarySource::Download),
    };
    Ok((provisioning, seen.sha))
}

/// Identify, resolve, push, install (plan Phase 7 §3). Nothing upgrades in
/// place and the server never self-updates: the client drives.
pub async fn provision(
    host: &Host,
    app_data_dir: &Path,
    on_step: impl Fn(ProvisionStep),
    ssh: &Ssh,
) -> Result<(), AppError> {
    let started = std::time::Instant::now();
    let current = std::sync::Mutex::new(ProvisionStep::Probe);
    let result = install(host, app_data_dir, ssh, |step| {
        log::info!(
            "installing hyprlayer-server {SERVER_VERSION} on {}: {step:?}",
            host.destination
        );
        *current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = step;
        on_step(step);
    })
    .await;
    let step = *current
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match &result {
        Ok(()) => log::info!(
            "installed hyprlayer-server {SERVER_VERSION} on {} in {:?}",
            host.destination,
            started.elapsed()
        ),
        Err(e) => log::error!(
            "installing on {} failed at {step:?}: {}: {}",
            host.destination,
            e.code,
            e.message
        ),
    }
    result
}

async fn install(
    host: &Host,
    app_data_dir: &Path,
    ssh: &Ssh,
    on_step: impl Fn(ProvisionStep),
) -> Result<(), AppError> {
    on_step(ProvisionStep::Probe);
    let uname = supervisor::ssh_capture(host, "uname -sm", ssh).await?;
    let triple = supervisor::triple_for_uname(host, &uname)?;

    let local = match resolve_local(app_data_dir, exe_dir().as_deref(), triple) {
        Some(found) => {
            log::info!(
                "using the {triple} binary from {:?}: {}",
                found.source,
                found.path.display()
            );
            found.path
        }
        None => download_into_cache(app_data_dir, triple, &on_step).await?,
    };
    let bytes = std::fs::metadata(&local)
        .map_err(|e| AppError::internal(format!("could not read {}: {e}", local.display())))?
        .len();

    on_step(ProvisionStep::Upload);
    supervisor::ssh_send_file(host, &stage_script(SERVER_VERSION), &local, ssh).await?;
    on_step(ProvisionStep::Install);
    supervisor::ssh_capture(host, &commit_script(SERVER_VERSION, bytes), ssh).await?;
    on_step(ProvisionStep::Done);
    Ok(())
}

fn download_error(triple: &str, detail: impl std::fmt::Display) -> AppError {
    AppError::server_download_failed(SERVER_VERSION, triple, detail.to_string())
}

async fn download_into_cache(
    app_data_dir: &Path,
    triple: &str,
    on_step: &impl Fn(ProvisionStep),
) -> Result<PathBuf, AppError> {
    let asset = asset_name(triple);
    let dest = cache_path(app_data_dir, triple);
    let parent = dest.parent().expect("the cache path has a parent");
    std::fs::create_dir_all(parent).map_err(|e| {
        download_error(
            triple,
            format!("could not create {}: {e}", parent.display()),
        )
    })?;
    let part = dest.with_extension("part");

    on_step(ProvisionStep::Download);
    let base = format!("{RELEASE_BASE}/server-v{SERVER_VERSION}");
    let binary_url = format!("{base}/{asset}");
    curl_to(&binary_url, &part)
        .await
        .map_err(|e| download_error(triple, format!("{binary_url}: {e}")))?;

    on_step(ProvisionStep::Verify);
    let sums_url = format!("{base}/{SUMS_ASSET}");
    let sums = match curl_text(&sums_url).await {
        Ok(sums) => sums,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(download_error(triple, format!("{sums_url}: {e}")));
        }
    };
    let Some(expected) = expected_sha(&sums, &asset) else {
        let _ = std::fs::remove_file(&part);
        return Err(download_error(
            triple,
            format!("{SUMS_ASSET} lists no checksum for {asset}"),
        ));
    };
    verify_and_store(&part, &dest, expected)?;
    Ok(dest)
}

fn expected_sha<'a>(sums: &'a str, asset: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let (sha, name) = line.split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches('*') == asset).then_some(sha.trim())
    })
}

/// A binary about to run on someone else's machine is not installed
/// unverified: a mismatch deletes the partial download and writes nothing, so
/// the cache can never hold bytes that failed their checksum.
fn verify_and_store(part: &Path, dest: &Path, expected: &str) -> Result<(), AppError> {
    let bytes = std::fs::read(part)
        .map_err(|e| AppError::internal(format!("could not read {}: {e}", part.display())))?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if !actual.eq_ignore_ascii_case(expected.trim()) {
        let _ = std::fs::remove_file(part);
        return Err(AppError::internal(format!(
            "The downloaded hyprlayer-server {SERVER_VERSION} did not match its published checksum \
             (expected {expected}, got {actual}); it was discarded."
        )));
    }
    std::fs::rename(part, dest)
        .map_err(|e| AppError::internal(format!("could not write {}: {e}", dest.display())))
}

/// ADR-0008 keeps an HTTP client out of the desktop's manifest, and this call
/// belongs to no core service, so it shells out the way ADR-0005 does for the
/// network half of git. The timeouts are why a cold cache offline reports
/// rather than hangs.
fn curl(url: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("curl");
    crate::supervisor::hide_console(&mut cmd);
    cmd.arg("-fsSL")
        .arg("--proto")
        .arg("=https")
        .arg("--connect-timeout")
        .arg(CONNECT_TIMEOUT)
        .arg("--max-time")
        .arg(DOWNLOAD_TIMEOUT)
        .arg(url)
        .kill_on_drop(true)
        .stdin(Stdio::null());
    cmd
}

async fn curl_output(mut cmd: tokio::process::Command) -> Result<Vec<u8>, String> {
    let output = cmd
        .output()
        .await
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        return Err(if stderr.is_empty() {
            format!("curl exited {}", output.status)
        } else {
            stderr.to_string()
        });
    }
    Ok(output.stdout)
}

async fn curl_to(url: &str, dest: &Path) -> Result<(), String> {
    let mut cmd = curl(url);
    cmd.arg("-o").arg(dest);
    curl_output(cmd).await.map(|_| ())
}

async fn curl_text(url: &str) -> Result<String, String> {
    let bytes = curl_output(curl(url)).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triple() -> &'static str {
        "x86_64-unknown-linux-gnu"
    }

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn a_staged_cache_file_wins_over_a_download() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        write(&cache_path(&data, triple()), "staged");

        let found = resolve_local(&data, None, triple()).expect("the cache is a hit");
        assert_eq!(found.source, BinarySource::Cache);
        assert_eq!(found.path, cache_path(&data, triple()));
    }

    #[test]
    fn a_binary_beside_the_app_is_used_when_the_cache_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let beside = dir.path().join("bin");
        write(&beside.join(asset_name(triple())), "bundled");

        let found = resolve_local(&data, Some(&beside), triple()).expect("the sibling is a hit");
        assert_eq!(found.source, BinarySource::Beside);
    }

    #[test]
    fn the_cache_wins_over_a_binary_beside_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let beside = dir.path().join("bin");
        write(&cache_path(&data, triple()), "staged");
        write(&beside.join(asset_name(triple())), "bundled");

        let found = resolve_local(&data, Some(&beside), triple()).unwrap();
        assert_eq!(found.source, BinarySource::Cache);
    }

    #[test]
    fn a_download_is_attempted_only_when_neither_exists() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let beside = dir.path().join("bin");
        std::fs::create_dir_all(&beside).unwrap();

        assert!(resolve_local(&data, Some(&beside), triple()).is_none());
    }

    #[test]
    fn an_interrupted_download_is_not_mistaken_for_a_cached_binary() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        write(&cache_path(&data, triple()).with_extension("part"), "half");

        assert!(resolve_local(&data, None, triple()).is_none());
    }

    #[test]
    fn a_checksum_mismatch_refuses_the_install_rather_than_writing() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("hyprlayer-server.part");
        let dest = dir.path().join("hyprlayer-server");
        std::fs::write(&part, b"tampered").unwrap();

        let err = verify_and_store(&part, &dest, &"0".repeat(64)).expect_err("the sha differs");
        assert!(err.message.contains("checksum"));
        assert!(!dest.exists(), "nothing is written on a mismatch");
        assert!(!part.exists(), "the partial download is discarded");
    }

    #[test]
    fn a_matching_checksum_stores_the_download() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("hyprlayer-server.part");
        let dest = dir.path().join("hyprlayer-server");
        std::fs::write(&part, b"bytes").unwrap();
        let sha = format!("{:x}", Sha256::digest(b"bytes"));

        verify_and_store(&part, &dest, &sha).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"bytes");
    }

    #[test]
    fn the_checksum_file_is_read_by_asset_name() {
        let sums = "aaa  hyprlayer-server-aarch64-apple-darwin\nbbb  hyprlayer-server-x86_64-unknown-linux-gnu\n";
        assert_eq!(expected_sha(sums, &asset_name(triple())), Some("bbb"));
        assert_eq!(expected_sha(sums, "hyprlayer-server-nope"), None);
    }

    #[test]
    fn an_interrupted_push_leaves_no_directory_a_later_run_reads_as_installed() {
        let staged = stage_script("0.7.5");
        let committed = commit_script("0.7.5", 42);

        // The push only ever writes under `<version>.tmp`; the version
        // directory is created by the rename in the second round trip.
        assert!(staged.contains("0.7.5.tmp"));
        assert!(!staged.contains("mv "));
        assert!(committed.contains("mv \"$d\" \"$r/0.7.5\""));
        assert!(committed.contains("-eq 42"));
        // And a staging directory is never reported as an installed version.
        let seen = parse_inspect("Linux x86_64\n@version 0.7.4\n@version 0.7.5.tmp\n");
        assert_eq!(seen.versions, vec!["0.7.4".to_string()]);
    }

    #[test]
    fn the_relay_runs_the_version_keyed_path_not_whatever_path_resolves() {
        let command = relay_command("0.8.2");
        assert!(command.starts_with("\"$HOME/.hyprlayer/server/"));
        assert!(command.ends_with("/0.8.2/hyprlayer-server\" relay"));
    }

    #[test]
    fn a_missing_current_version_falls_back_to_an_older_usable_install() {
        let installed = vec![
            "0.0.1".to_string(),
            server_version::MIN_SERVER_VERSION.to_string(),
        ];
        assert_eq!(
            usable_version(HostServerState::Missing, &installed).as_deref(),
            Some(server_version::MIN_SERVER_VERSION)
        );
        assert_eq!(
            usable_version(HostServerState::Missing, &["0.0.1".to_string()]),
            None
        );
    }

    #[test]
    fn a_server_that_keeps_answering_fails_the_stop() {
        let script = stop_script("0.8.3");
        // Only "nothing is running" short-circuits to success.
        assert!(script.starts_with("\"$HOME/.hyprlayer/server/0.8.3/hyprlayer-server\" status"));
        assert!(script.contains("|| exit 0;"));
        assert!(!script.contains("stop >/dev/null 2>&1 || exit 0"));
        assert!(script.contains("exit 1"));
    }

    #[test]
    fn a_stale_current_version_does_not_fall_back() {
        let installed = vec![server_version::MIN_SERVER_VERSION.to_string()];
        assert_eq!(usable_version(HostServerState::Stale, &installed), None);
        assert_eq!(
            usable_version(HostServerState::Ready, &installed).as_deref(),
            Some(SERVER_VERSION)
        );
    }

    #[test]
    fn inspect_output_separates_the_uname_from_the_markers() {
        let seen = parse_inspect("Darwin arm64\n@installed\n@sha abc123\n@version 0.7.5\n");
        assert_eq!(seen.uname, "Darwin arm64");
        assert!(seen.installed);
        assert_eq!(seen.sha.as_deref(), Some("abc123"));
        assert_eq!(seen.versions, vec!["0.7.5".to_string()]);

        let bare = parse_inspect("Linux x86_64\n");
        assert!(!bare.installed);
        assert!(bare.sha.is_none());
        assert!(bare.versions.is_empty());

        // A host with no sha tool emits the marker with nothing after it.
        let unhashable = parse_inspect("Linux x86_64\n@installed\n@sha \n");
        assert!(unhashable.installed);
        assert!(unhashable.sha.is_none());
    }

    #[test]
    fn a_host_binary_with_different_bytes_is_stale_though_the_version_matches() {
        assert_eq!(
            state_for(true, Some("aaa"), Some("bbb")),
            HostServerState::Stale
        );
    }

    #[test]
    fn an_equal_sha_installs_nothing() {
        assert_eq!(
            state_for(true, Some("aaa"), Some("AAA")),
            HostServerState::Ready
        );
    }

    #[test]
    fn a_local_binary_that_would_need_a_download_trusts_the_version() {
        // `None` is exactly the "we would have to fetch 16 MB to compare" case;
        // it must not be treated as a mismatch, and nothing fetches it.
        assert_eq!(state_for(true, Some("aaa"), None), HostServerState::Ready);
        assert_eq!(state_for(false, None, None), HostServerState::Missing);
    }
}
