//! Typed wrappers around `hyprlayer` CLI invocations.
//!
//! All machine-readable commands the app uses go through here so the
//! CLI surface is mocked/tested in one place. cwd matters: `storage info`
//! resolves repo mappings against the process working directory.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::process::binaries::{BinaryKind, BinaryResolver};
use crate::process::{Exec, ExecOutput};

/// Backend settings from `storage info --json`. The CLI emits a different
/// key set per backend kind; this is the union, all optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct StorageSettings {
    // git
    pub thoughts_repo: Option<String>,
    pub repos_dir: Option<String>,
    pub global_dir: Option<String>,
    // obsidian
    pub vault_path: Option<String>,
    pub vault_subpath: Option<String>,
    pub content_root: Option<String>,
    // notion
    pub parent_page_id: Option<String>,
    pub database_id: Option<String>,
    // anytype
    pub space_id: Option<String>,
    pub type_id: Option<String>,
    pub api_token_env: Option<String>,
}

/// One field of the 10-field thought metadata schema
/// (`hyprlayer-cli/src/backends/schema.rs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SchemaFieldInfo {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    pub required: bool,
}

/// Payload of `hyprlayer storage info --json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    pub backend: String,
    #[serde(default)]
    pub settings: StorageSettings,
    #[serde(default)]
    pub project_path: String,
    #[serde(default)]
    pub mapped_name: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub schema: Vec<SchemaFieldInfo>,
}

/// Typed response for `hyprlayer ai status --json`.
///
/// The CLI emits a per-tool block plus the two bundle-freshness fields below
/// (`ai/status.rs`); anything else it adds is absorbed by `#[serde(default)]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct AiStatus {
    /// The agent tool name as reported by the CLI, e.g. `"Claude Code"`.
    pub agent_tool: Option<String>,
    /// Whether the agent tool is installed and ready to use.
    pub installed: bool,
    /// Where the tool's bundle is installed, tilde-abbreviated for display
    /// (`~/.claude/`) — `AgentTool::dest_display`, not a binary path and not
    /// a path to hand to the filesystem.
    pub location: Option<String>,
    /// `master` HEAD of BrightBlock/hyprlayer-cli that the installed agent
    /// bundle was pinned to. `None` before the first install, or when ref
    /// resolution was unreachable at install time (the CLI still installs).
    pub agents_installed_sha: Option<String>,
    /// Unix seconds of the CLI's last 24h auto-check. `None` right after an
    /// explicit install: `record_install` clears it so the next check re-runs.
    pub last_agent_check: Option<f64>,
    /// Version of the bundle actually on disk, as the CLI recorded it at
    /// install time (`agentsInstalledVersion` in its config). Added by CLI
    /// 1.6.0; `None` against an older binary, and against a bundle installed
    /// before versioned assets existed.
    ///
    /// NOT proof of what the files are: `ai reinstall --version` records the
    /// version it was *asked* for even when its source chain fell through to
    /// master's tree, which is the trap [`crate::agent_bundle::reinstall_source`]
    /// exists to catch.
    pub assets_version: Option<String>,
    /// Version of the `hyprlayer` binary itself. The bundle it ships with,
    /// and therefore what `--unpin` installs.
    pub binary_version: Option<String>,
    /// The pin, if one is set (`agentsPinnedVersion`). `None` means unpinned:
    /// the binary's own bundle tracks the binary. A pin that differs from
    /// `binary_version` is the state a user gets stuck in, so the UI says so.
    pub pinned_version: Option<String>,
}

/// One selectable bundle version from `hyprlayer ai versions --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AiVersionEntry {
    pub version: String,
    /// RFC 3339, as GitHub's release API spells it. Optional rather than
    /// required: it is display sugar, and a release missing the field must
    /// not cost the user the whole list.
    #[serde(default)]
    pub published_at: Option<String>,
    /// The version currently installed.
    #[serde(default)]
    pub current: bool,
    /// The version the config pins to.
    #[serde(default)]
    pub pinned: bool,
}

/// Payload of `hyprlayer ai versions --json`.
///
/// The CLI has already filtered `versions` to releases that carry a bundle
/// for the active harness — a release with no verifiable asset is absent, not
/// present-and-flagged. That filtering is the whole reason the picker is
/// populated from here and never from a version string assembled anywhere
/// else; see [`crate::agent_bundle::reinstall_source`] for what installing an
/// unlisted version actually does.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct AiVersions {
    pub assets_version: Option<String>,
    pub binary_version: Option<String>,
    /// The harness the list is for (`"claude"`), following the configured
    /// agent tool. One list, not one per harness — the CLI reports a single
    /// active harness and the app follows it.
    pub harness: Option<String>,
    pub pinned_version: Option<String>,
    pub versions: Vec<AiVersionEntry>,
}

/// What `hyprlayer self-update --check` reported.
///
/// `latest` is `Some` only when the CLI itself judged a newer version
/// installable, so it doubles as "an update is available". Everything `None`
/// means no answer — the check exited non-zero (an install method whose
/// source is unreachable, which is what AUR does with no helper installed) or
/// its wording moved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct CliUpdateCheck {
    pub current: Option<String>,
    pub latest: Option<String>,
    /// The CLI's own upgrade command for the detected install method, e.g.
    /// `brew upgrade hyprlayer`. Shown verbatim when the in-app update fails,
    /// which is the only way a user on an interactive package manager gets
    /// the command they actually need.
    pub hint: Option<String>,
}

/// Parse `hyprlayer self-update --check`'s stdout.
///
/// Both lines are verbatim from `commands/self_update.rs` and identical in
/// 1.5.9 (4c5cf37) and 1.6.0 (9a2a33c) — one parse covers every CLI the app
/// can meet:
///
/// ```text
/// Update available: 1.5.9 → 1.6.2 (brew upgrade hyprlayer)
/// hyprlayer is up to date (1.6.0).
/// ```
///
/// The separator is U+2192, not `->`. Anything unrecognized returns the empty
/// check rather than a guess: this drives an Update button, and inventing a
/// version to offer would be worse than offering nothing.
pub fn parse_self_update_check(stdout: &str) -> CliUpdateCheck {
    for line in stdout.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Update available: ") {
            // `rsplit_once` on the LAST " (": the hint is a shell command and
            // a version never contains a parenthesis, so the tail is the hint
            // even if a future hint gains one of its own.
            let Some((versions, hint)) = rest.rsplit_once(" (") else {
                continue;
            };
            let Some(hint) = hint.strip_suffix(')') else {
                continue;
            };
            let Some((current, latest)) = versions.split_once(" \u{2192} ") else {
                continue;
            };
            return CliUpdateCheck {
                current: Some(current.trim().to_string()),
                latest: Some(latest.trim().to_string()),
                hint: Some(hint.trim().to_string()),
            };
        }
        if let Some(rest) = line.strip_prefix("hyprlayer is up to date (") {
            if let Some(current) = rest.strip_suffix(").") {
                return CliUpdateCheck {
                    current: Some(current.trim().to_string()),
                    ..Default::default()
                };
            }
        }
    }
    CliUpdateCheck::default()
}

/// Which bundle `hyprlayer ai reinstall` should install.
///
/// An enum rather than two optional arguments because `--version` and
/// `--unpin` are mutually exclusive at the clap layer: passing both makes the
/// CLI exit non-zero, and a pair of `Option`s can express exactly that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleSelection<'a> {
    /// `--version <v>`: pin to this version and install it.
    Version(&'a str),
    /// `--unpin`: drop the pin and install the binary's own bundle.
    Unpin,
}

pub struct HyprCli<'a> {
    resolver: &'a BinaryResolver,
}

impl<'a> HyprCli<'a> {
    pub fn new(resolver: &'a BinaryResolver) -> Self {
        Self { resolver }
    }

    /// The one spawn contract for every `hyprlayer` invocation: the resolved
    /// binary, the augmented PATH, the caller's cwd and timeout. Kept single
    /// so [`exec`](Self::exec) and [`ai_reinstall_selected`](Self::ai_reinstall_selected)
    /// cannot drift — they differ only in `run()` vs `run_unchecked()`.
    async fn command(&self, cwd: &Path, args: &[&str], timeout: Duration) -> Result<Exec> {
        let bin = self.resolver.require(BinaryKind::Hyprlayer).await?;
        let path = self.resolver.augmented_path().await;
        Ok(Exec::new(bin)
            .args(args.iter().copied())
            .cwd(cwd)
            .env("PATH", path)
            .timeout(timeout))
    }

    async fn exec(&self, cwd: &Path, args: &[&str], timeout: Duration) -> Result<String> {
        let output = self.command(cwd, args, timeout).await?.run().await?;
        Ok(output.stdout)
    }

    /// `hyprlayer storage info --json`, run from `repo_path`. Side effect we
    /// rely on at startup: this loads the config through the CLI's migration
    /// path, so the file on disk is guaranteed current-schema afterwards.
    pub async fn storage_info(&self, repo_path: &Path) -> Result<StorageInfo> {
        let stdout = self
            .exec(
                repo_path,
                &["storage", "info", "--json"],
                Duration::from_secs(30),
            )
            .await?;
        serde_json::from_str(&stdout).map_err(|e| CoreError::OutputParse {
            program: "hyprlayer storage info".into(),
            detail: e.to_string(),
        })
    }

    /// `hyprlayer thoughts sync [-m message]`, run from the repo.
    pub async fn thoughts_sync(&self, repo_path: &Path, message: Option<&str>) -> Result<String> {
        let mut args = vec!["thoughts", "sync"];
        if let Some(m) = message {
            args.push("-m");
            args.push(m);
        }
        // Network-bound (pull/push): generous timeout.
        self.exec(repo_path, &args, Duration::from_secs(300)).await
    }

    /// `hyprlayer thoughts init --yes --directory <name> [--profile <p>]`.
    /// Precondition (CLI rule): a config file must already exist.
    pub async fn thoughts_init(
        &self,
        repo_path: &Path,
        directory: &str,
        profile: Option<&str>,
    ) -> Result<String> {
        let mut args = vec!["thoughts", "init", "--yes", "--directory", directory];
        if let Some(p) = profile {
            args.push("--profile");
            args.push(p);
        }
        self.exec(repo_path, &args, Duration::from_secs(120)).await
    }

    /// `hyprlayer thoughts uninit`, run from a repo or worktree: removes its
    /// `thoughts/` links and drops the cwd's repoMappings entry. Fully
    /// non-interactive (verified against the CLI source).
    pub async fn thoughts_uninit(&self, repo_path: &Path) -> Result<String> {
        self.exec(repo_path, &["thoughts", "uninit"], Duration::from_secs(60))
            .await
    }

    /// `hyprlayer thoughts profile create <name> --repo … --repos-dir … --global-dir …`.
    /// Every flag is always passed: an omitted one makes the CLI prompt, and
    /// with no terminal attached dialoguer errors out instead. Not repo-scoped
    /// (the config is global), any existing `cwd` works.
    pub async fn thoughts_profile_create(
        &self,
        cwd: &Path,
        name: &str,
        repo: &str,
        repos_dir: &str,
        global_dir: &str,
    ) -> Result<String> {
        self.exec(
            cwd,
            &[
                "thoughts",
                "profile",
                "create",
                name,
                "--repo",
                repo,
                "--repos-dir",
                repos_dir,
                "--global-dir",
                global_dir,
            ],
            Duration::from_secs(30),
        )
        .await
    }

    /// `hyprlayer ai status --json`, typed into [`AiStatus`]; extra CLI
    /// fields are silently ignored.
    ///
    /// Emits a bare `{}` when no config exists, when `ai` is unset, or when
    /// `ai.agentTool` is unset (`hyprlayer-cli/src/commands/ai/status.rs`,
    /// `print_not_configured`), which `#[serde(default)]` turns into
    /// `AiStatus::default()` — `installed: false`, every option `None`.
    /// Callers must read `installed` rather than treating a `None`
    /// `agents_installed_sha` as "a bundle is installed but unpinned".
    pub async fn ai_status(&self, cwd: &Path) -> Result<AiStatus> {
        let stdout = self
            .exec(cwd, &["ai", "status", "--json"], Duration::from_secs(30))
            .await?;
        serde_json::from_str(&stdout).map_err(|e| CoreError::OutputParse {
            program: "hyprlayer ai status".into(),
            detail: e.to_string(),
        })
    }

    /// `hyprlayer ai reinstall`. Fully non-interactive (verified against
    /// `hyprlayer-cli/src/commands/ai/reinstall.rs` — no dialoguer prompts),
    /// so it runs headless rather than needing a PTY. Network-bound: a
    /// tarball download, with a per-file GitHub API walk as the CLI's own
    /// fallback.
    ///
    /// The `Ok` value is the CLI's stdout. A failure does NOT carry it:
    /// `Exec::run` builds `CoreError::Exec` from `stderr_tail` and drops
    /// stdout, so a caller wanting the CLI's own words on the error path has
    /// only stderr.
    pub async fn ai_reinstall(&self, cwd: &Path) -> Result<String> {
        self.exec(cwd, &["ai", "reinstall"], Duration::from_secs(300))
            .await
    }

    /// `hyprlayer ai versions --json --limit <n>`, the list a version picker
    /// may offer. Requires network: it queries the GitHub releases API.
    ///
    /// `limit` is how many recent *releases* to examine, not how many results
    /// come back — releases carrying no bundle for the active harness are
    /// examined and then dropped, so a limit of 10 can answer with three.
    pub async fn ai_versions(&self, cwd: &Path, limit: u32) -> Result<AiVersions> {
        let limit = limit.to_string();
        let stdout = self
            .exec(
                cwd,
                &["ai", "versions", "--json", "--limit", &limit],
                // Two GitHub API round trips on a cold cache, against the
                // 8s used for the single ref-advertisement probe.
                Duration::from_secs(30),
            )
            .await?;
        serde_json::from_str(&stdout).map_err(|e| CoreError::OutputParse {
            program: "hyprlayer ai versions".into(),
            detail: e.to_string(),
        })
    }

    /// `hyprlayer ai reinstall --version <v>` or `--unpin`.
    ///
    /// Returns the raw [`ExecOutput`] — stdout, stderr and exit code — rather
    /// than `Result<String>` like [`ai_reinstall`](Self::ai_reinstall), and
    /// runs `run_unchecked` so a non-zero exit is data instead of an error.
    /// Both halves are load-bearing for a *selected* version: `Exec::run`'s
    /// `CoreError::Exec` keeps only a stderr tail and drops stdout entirely,
    /// and stdout is where the CLI reports which source it actually installed
    /// from. A reinstall that exits 0 having silently fallen back to master's
    /// tree is the failure mode the caller has to catch, and it is invisible
    /// from the exit code alone — see [`crate::agent_bundle::reinstall_source`].
    pub async fn ai_reinstall_selected(
        &self,
        cwd: &Path,
        selection: BundleSelection<'_>,
    ) -> Result<ExecOutput> {
        let mut args = vec!["ai", "reinstall"];
        match selection {
            BundleSelection::Version(v) => {
                args.push("--version");
                args.push(v);
            }
            BundleSelection::Unpin => args.push("--unpin"),
        }
        self.command(cwd, &args, Duration::from_secs(300))
            .await?
            .run_unchecked()
            .await
    }

    /// `hyprlayer self-update --check`. Network-bound, and exits non-zero when
    /// the install method's source cannot be reached — so it runs unchecked
    /// and a failure reads as "no answer" rather than an error the user has to
    /// dismiss.
    pub async fn self_update_check(&self, cwd: &Path) -> Result<CliUpdateCheck> {
        let output = self
            .command(cwd, &["self-update", "--check"], Duration::from_secs(30))
            .await?
            .run_unchecked()
            .await?;
        if output.exit_code != Some(0) {
            return Ok(CliUpdateCheck::default());
        }
        Ok(parse_self_update_check(&output.stdout))
    }

    /// `hyprlayer self-update`, which dispatches to the detected package
    /// manager.
    ///
    /// `run_unchecked`, because a failure here is the interesting case: the
    /// CLI shells `brew`/`winget`/`scoop`/an AUR helper with inherited stdio,
    /// and `Exec` gives the child `/dev/null` on stdin — so a manager that
    /// wants to prompt gets EOF and dies quickly instead of hanging out the
    /// timeout. The user then needs the manager's own words and the hint
    /// command, which only survive as data.
    pub async fn self_update(&self, cwd: &Path) -> Result<ExecOutput> {
        self.command(cwd, &["self-update"], Duration::from_secs(300))
            .await?
            .run_unchecked()
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim from 1.6.0 (9a2a33c); 1.5.9 prints the same two lines.
    #[test]
    fn self_update_check_reads_an_available_update() {
        let got = parse_self_update_check(
            "Update available: 1.5.9 \u{2192} 1.6.2 (brew upgrade hyprlayer)\n",
        );
        assert_eq!(got.current.as_deref(), Some("1.5.9"));
        assert_eq!(got.latest.as_deref(), Some("1.6.2"));
        assert_eq!(got.hint.as_deref(), Some("brew upgrade hyprlayer"));
    }

    #[test]
    fn self_update_check_reads_up_to_date_as_no_update() {
        let got = parse_self_update_check("hyprlayer is up to date (1.6.0).\n");
        assert_eq!(got.current.as_deref(), Some("1.6.0"));
        assert_eq!(got.latest, None);
    }

    #[test]
    fn self_update_check_offers_nothing_it_cannot_read() {
        // The AUR failure path writes to stderr and exits 1, so stdout is
        // empty; a reworded line must land here too, not on a guess.
        assert_eq!(parse_self_update_check(""), CliUpdateCheck::default());
        assert_eq!(
            parse_self_update_check("Update available: 1.5.9 -> 1.6.2 (brew upgrade hyprlayer)"),
            CliUpdateCheck::default()
        );
        assert_eq!(
            parse_self_update_check("Update available: 1.5.9 \u{2192} 1.6.2"),
            CliUpdateCheck::default()
        );
    }

    /// Real `hyprlayer ai status --json` from 1.6.0 (9a2a33c), verbatim.
    const STATUS_1_6_0: &str = r#"{
      "agentTool": "Claude Code",
      "agentsInstalledSha": "9a2a33c6fc0648e8cd7bac278365c8a9dcca220b",
      "assetsVersion": "1.6.0",
      "binaryVersion": "1.6.0",
      "installed": true,
      "lastAgentCheck": null,
      "location": "~/.claude/",
      "pinnedVersion": "1.6.0"
    }"#;

    /// Real `hyprlayer ai versions --json` from the same binary, verbatim.
    const VERSIONS_1_6_0: &str = r#"{
      "assetsVersion": "1.6.0",
      "binaryVersion": "1.6.0",
      "harness": "claude",
      "pinnedVersion": "1.6.0",
      "versions": [
        { "version": "1.6.0",      "publishedAt": "2026-08-24T04:13:17Z", "current": true,  "pinned": true  },
        { "version": "1.6.0-rc.2", "publishedAt": "2026-08-24T03:17:01Z", "current": false, "pinned": false },
        { "version": "1.6.0-rc.1", "publishedAt": "2026-08-24T01:06:31Z", "current": false, "pinned": false }
      ]
    }"#;

    #[test]
    fn parses_the_1_6_0_status_payload() {
        let s: AiStatus = serde_json::from_str(STATUS_1_6_0).unwrap();
        assert!(s.installed);
        assert_eq!(s.agent_tool.as_deref(), Some("Claude Code"));
        assert_eq!(s.assets_version.as_deref(), Some("1.6.0"));
        assert_eq!(s.binary_version.as_deref(), Some("1.6.0"));
        assert_eq!(s.pinned_version.as_deref(), Some("1.6.0"));
    }

    /// `pinnedVersion: null` is the unpinned state, and it is a real value the
    /// CLI emits — not a missing key. Distinguishing "no pin" from "pinned to
    /// something" is the whole point of the field, so a null that deserialized
    /// into anything but `None` would invert the UI's headline claim.
    #[test]
    fn reads_a_null_pin_as_unpinned() {
        let s: AiStatus = serde_json::from_str(
            r#"{"agentTool":"Claude Code","installed":true,"assetsVersion":"1.6.0","binaryVersion":"1.6.0","pinnedVersion":null}"#,
        )
        .unwrap();
        assert_eq!(s.pinned_version, None);
        assert_eq!(s.assets_version.as_deref(), Some("1.6.0"));
    }

    /// A pre-1.6.0 binary emits none of the three version keys. It must still
    /// deserialize — the app has to run against whatever `hyprlayer` is on the
    /// machine, and the version row simply has nothing to say.
    #[test]
    fn tolerates_a_pre_1_6_0_status_payload() {
        let s: AiStatus = serde_json::from_str(
            r#"{"agentTool":"Claude Code","installed":true,"location":"~/.claude/","agentsInstalledSha":"abc","lastAgentCheck":null}"#,
        )
        .unwrap();
        assert!(s.installed);
        assert_eq!(s.assets_version, None);
        assert_eq!(s.binary_version, None);
        assert_eq!(s.pinned_version, None);
    }

    /// The CLI answers a bare `{}` when nothing is configured
    /// (`print_not_configured`), which must not be a parse error.
    #[test]
    fn reads_an_unconfigured_status_as_not_installed() {
        let s: AiStatus = serde_json::from_str("{}").unwrap();
        assert!(!s.installed);
        assert_eq!(s.pinned_version, None);
    }

    #[test]
    fn parses_the_1_6_0_versions_payload() {
        let v: AiVersions = serde_json::from_str(VERSIONS_1_6_0).unwrap();
        assert_eq!(v.harness.as_deref(), Some("claude"));
        assert_eq!(v.binary_version.as_deref(), Some("1.6.0"));
        assert_eq!(v.pinned_version.as_deref(), Some("1.6.0"));
        assert_eq!(v.versions.len(), 3);
        assert_eq!(v.versions[0].version, "1.6.0");
        assert_eq!(
            v.versions[0].published_at.as_deref(),
            Some("2026-08-24T04:13:17Z")
        );
        assert!(v.versions[0].current && v.versions[0].pinned);
        // Prereleases are listed, and neither current nor pinned.
        assert_eq!(v.versions[2].version, "1.6.0-rc.1");
        assert!(!v.versions[2].current && !v.versions[2].pinned);
    }

    /// No release in the examined window carries a bundle. Distinct from an
    /// error: the CLI reached GitHub and the honest answer is "nothing to
    /// offer", so the picker has to render empty rather than fail.
    #[test]
    fn parses_an_empty_versions_list() {
        let v: AiVersions = serde_json::from_str(
            r#"{"assetsVersion":"1.6.0","binaryVersion":"1.6.0","harness":"claude","pinnedVersion":null,"versions":[]}"#,
        )
        .unwrap();
        assert!(v.versions.is_empty());
        assert_eq!(v.pinned_version, None);
        assert_eq!(v.harness.as_deref(), Some("claude"));
    }

    /// `publishedAt` is display sugar; a release missing it must cost that one
    /// row its date, not the whole list.
    #[test]
    fn keeps_an_entry_whose_published_date_is_absent() {
        let v: AiVersions =
            serde_json::from_str(r#"{"versions":[{"version":"1.6.0","current":true}]}"#).unwrap();
        assert_eq!(v.versions.len(), 1);
        assert_eq!(v.versions[0].published_at, None);
        assert!(v.versions[0].current);
        assert!(!v.versions[0].pinned);
    }
}
