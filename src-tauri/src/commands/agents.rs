//! IPC surface for Claude Code agent definitions. Thin by design (ADR-0001):
//! discovery, parsing and rendering live in `hyprlayer_core::agents`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tauri::State;

use hyprlayer_core::agent_bundle::{self, AgentChange, BundleSource};
use hyprlayer_core::agents::{self, AgentContextFile, AgentFile, AgentMcpServer, AgentOverlay};
use hyprlayer_core::hyprcli::{AiVersions, BundleSelection, HyprCli};
use hyprlayer_core::skills::AgentSkill;

use crate::commands::blocking;
use crate::error::{AppError, CmdResult};
use crate::state::AppState;

/// Every agent visible from `project_path`, that project's own plus the user's.
#[tauri::command]
#[specta::specta]
pub async fn list_agent_files(project_path: Option<String>) -> CmdResult<Vec<AgentFile>> {
    blocking("list_agent_files", move || {
        Ok(agents::list_agents(project_path.as_deref().map(Path::new)))
    })
    .await?
}

/// Write `agent` back to its own `.md` file, returning it as re-read from disk.
#[tauri::command]
#[specta::specta]
pub async fn save_agent_file(agent: AgentFile) -> CmdResult<AgentFile> {
    blocking("save_agent_file", move || {
        agents::save_agent(&agent).map_err(AppError::from)
    })
    .await?
}

/// Delete an agent definition, the file IS the agent.
#[tauri::command]
#[specta::specta]
pub async fn delete_agent_file(path: String) -> CmdResult<()> {
    blocking("delete_agent_file", move || {
        agents::delete_agent(Path::new(&path)).map_err(AppError::from)
    })
    .await?
}

/// The path a new agent called `name` would occupy in `source`'s directory.
/// Returns `None` when that root doesn't apply (a project agent with no project).
#[tauri::command]
#[specta::specta]
pub async fn new_agent_file_path(
    name: String,
    source: String,
    project_path: Option<String>,
) -> CmdResult<Option<String>> {
    blocking("new_agent_file_path", move || {
        let dir: Option<PathBuf> =
            agents::agents_dir_for(project_path.as_deref().map(Path::new), &source);
        Ok(dir.map(|d| {
            d.join(format!("{}.md", agents::file_stem_for(&name)))
                .to_string_lossy()
                .into_owned()
        }))
    })
    .await?
}

/// Docs in the project worth appending to an agent's prompt.
#[tauri::command]
#[specta::specta]
pub async fn agent_context_files(project_path: Option<String>) -> CmdResult<Vec<AgentContextFile>> {
    blocking("agent_context_files", move || {
        Ok(agents::context_candidates(
            project_path.as_deref().map(Path::new),
        ))
    })
    .await?
}

/// MCP servers claude is configured with, for the Capabilities tab.
#[tauri::command]
#[specta::specta]
pub async fn agent_mcp_servers(project_path: Option<String>) -> CmdResult<Vec<AgentMcpServer>> {
    blocking("agent_mcp_servers", move || {
        Ok(agents::mcp_servers(project_path.as_deref().map(Path::new)))
    })
    .await?
}

/// Skills an agent's `skills:` key can name, for the Capabilities tab.
#[tauri::command]
#[specta::specta]
pub async fn agent_skills(project_path: Option<String>) -> CmdResult<Vec<AgentSkill>> {
    blocking("agent_skills", move || {
        Ok(hyprlayer_core::skills::list_skills(
            project_path.as_deref().map(Path::new),
        ))
    })
    .await?
}

/// Write a portable `.agent.json` to Downloads; returns where it landed.
#[tauri::command]
#[specta::specta]
pub async fn export_agent_file(agent: AgentFile, overlay: AgentOverlay) -> CmdResult<String> {
    blocking("export_agent_file", move || {
        agents::export_agent(&agent, &overlay).map_err(AppError::from)
    })
    .await?
}

// --- Agent bundle: freshness, update, restore ------------------------------
//
// The bundle in `~/.claude` — `agents`, `skills` and `settings.json` — is not
// baked into the `hyprlayer` binary: `hyprlayer ai reinstall` downloads it
// fresh from `BrightBlock/hyprlayer-cli@master` every time. Freshness is
// therefore a git SHA comparison the CLI already tracks
// (`agentsInstalledSha` in its config), but there is no check-only CLI
// command to ask "is a newer one available" without also installing it — so
// this file resolves `master`'s current SHA itself, the same way the CLI
// does (`agent_bundle::parse_master_sha`/`BUNDLE_REFS_URL`), and leaves the
// pure parsing in core per ADR-0001.

/// The one agent tool whose bundle this app can manage, as
/// `hyprlayer ai status --json` spells it (`AgentTool`'s `Display`).
///
/// `ai reinstall` installs into the *configured* tool's directory —
/// `~/.claude`, `<config>/Code/User` (Copilot) or `~/.config/opencode`
/// (`hyprlayer-cli/src/agents.rs`, `AgentTool::dest_dir`). The Agents view
/// reads `~/.claude/agents` and nothing else (`agents::agents_dir_for`), so
/// offering the update to a Copilot or OpenCode user would back up a tree the
/// reinstall never touches, rewrite one this app cannot see, and then report
/// "nothing changed". Gated rather than generalised: a second copy of
/// `dest_dir` here is a drift risk, and there is no view to put the other
/// tools' results in.
const MANAGEABLE_AGENT_TOOL: &str = "Claude Code";

/// `~/.claude`, the destination `hyprlayer ai reinstall` writes into for
/// [`MANAGEABLE_AGENT_TOOL`] (`hyprlayer-cli/src/agents.rs`,
/// `AgentTool::Claude::dest_dir`). Resolved here rather than in core:
/// home-directory resolution is IPC-layer plumbing, the same split
/// `get_ai_status`'s `cwd` already uses. Errors rather than falling back to a
/// bogus root (unlike a mere working directory, a wrong value here is a real
/// destination path).
fn claude_root() -> CmdResult<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".claude"))
        .ok_or_else(|| AppError::internal("could not determine the home directory"))
}

/// Timeout for the one-shot ref-advertisement probe, matching the
/// `models.rs`/`ollama.rs` `reqwest` precedent.
const BUNDLE_PROBE_TIMEOUT: Duration = Duration::from_secs(8);
/// Ref advertisements run a few KB; two orders of magnitude of headroom stops
/// a hostile or misconfigured source from streaming gigabytes into memory.
/// Mirrors the CLI's own `MAX_REFS_BYTES` (`hyprlayer-cli/src/agents.rs`).
const MAX_REFS_BYTES: usize = 1024 * 1024;

/// Resolve `master`'s current SHA from GitHub's smart-HTTP ref advertisement.
/// `None` on any failure — offline, DNS, a non-2xx response, an oversized or
/// unparseable body — a background freshness probe must never turn into a
/// dialog just because the user's wifi dropped.
async fn fetch_latest_bundle_sha() -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(BUNDLE_PROBE_TIMEOUT)
        .user_agent("hyprlayer-app")
        .build()
        .ok()?;
    let mut resp = client
        .get(agent_bundle::BUNDLE_REFS_URL)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    // Accumulated chunk by chunk against a running total, NOT `resp.bytes()`:
    // reqwest imposes no response-size limit of its own, so buffering first
    // and measuring afterwards would allocate exactly what MAX_REFS_BYTES
    // exists to refuse. Mirrors the CLI, which caps during the read
    // (`take(max_bytes + 1)`, `hyprlayer-cli/src/http.rs`).
    if resp
        .content_length()
        .is_some_and(|n| n > MAX_REFS_BYTES as u64)
    {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if body.len() + chunk.len() > MAX_REFS_BYTES {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    agent_bundle::parse_master_sha(&String::from_utf8_lossy(&body))
}

/// The agent bundle's freshness, for the Agents view's header chip.
#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentBundleStatus {
    /// Whether this app can manage the bundle at all: the CLI must report an
    /// installed [`MANAGEABLE_AGENT_TOOL`]. False for a user who has never
    /// run `hyprlayer ai configure` (the CLI answers `{}`, which deserializes
    /// to `installed: false`) and for one configured on another tool. The
    /// chip reads this before anything else — without it, a missing
    /// `installed_sha` looks exactly like a stale one and every unconfigured
    /// machine is told a new bundle is available.
    pub manageable: bool,
    /// SHA the installed bundle is pinned to, from the CLI's config.
    pub installed_sha: Option<String>,
    /// `master` HEAD as resolved just now. `None` when the probe failed —
    /// offline, or GitHub unreachable — which renders as "up to date" rather
    /// than as an error: not knowing is not the same as being behind. Also
    /// `None` with no probe run at all when `desired_version` is `Some`.
    pub latest_sha: Option<String>,
    /// `assetsVersion`.
    pub installed_version: Option<String>,
    /// `agent_bundle::desired_version`, and the label the update affordances
    /// name. `None` on a pre-1.6.0 CLI.
    pub desired_version: Option<String>,
    /// Unix seconds of the CLI's last auto-check — when the bundle was last
    /// *looked at*, not when it was last installed. `record_install` clears
    /// it, so it is `None` immediately after an update.
    pub last_checked: Option<f64>,
    pub update_available: bool,
}

/// `hyprlayer ai status --json`, and — only for a CLI too old to answer in
/// versions — a probe of GitHub's ref advertisement. See
/// [`agent_bundle::desired_version`] for why the SHA branch is the legacy one.
#[tauri::command]
#[specta::specta]
pub async fn agent_bundle_status(state: State<'_, AppState>) -> CmdResult<AgentBundleStatus> {
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let status = HyprCli::new(&state.binaries)
        .ai_status(&cwd)
        .await
        .map_err(AppError::from)?;
    let manageable =
        status.installed && status.agent_tool.as_deref() == Some(MANAGEABLE_AGENT_TOOL);
    let desired = agent_bundle::desired_version(
        status.pinned_version.as_deref(),
        status.binary_version.as_deref(),
    )
    .map(str::to_string);

    if !manageable {
        return Ok(AgentBundleStatus {
            manageable,
            installed_sha: status.agents_installed_sha,
            latest_sha: None,
            installed_version: status.assets_version,
            desired_version: desired,
            last_checked: status.last_agent_check,
            update_available: false,
        });
    }

    if let Some(desired) = desired {
        let update_available = agent_bundle::is_stale(status.assets_version.as_deref(), &desired);
        return Ok(AgentBundleStatus {
            manageable,
            installed_sha: status.agents_installed_sha,
            latest_sha: None,
            installed_version: status.assets_version,
            desired_version: Some(desired),
            last_checked: status.last_agent_check,
            update_available,
        });
    }

    let latest_sha = fetch_latest_bundle_sha().await;
    let update_available = latest_sha.as_deref().is_some_and(|latest| {
        agent_bundle::is_stale(status.agents_installed_sha.as_deref(), latest)
    });
    Ok(AgentBundleStatus {
        manageable,
        installed_sha: status.agents_installed_sha,
        latest_sha,
        installed_version: status.assets_version,
        desired_version: None,
        last_checked: status.last_agent_check,
        update_available,
    })
}

/// One agent the bundle update changed, for the report dialog.
#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentBundleChange {
    /// File stem, e.g. `"cartographer"` for `cartographer.md`. Also what
    /// `restore_agent_from_backup` takes.
    pub name: String,
    pub change: AgentChange,
    /// Absolute path to the agent's `.md` file, for display.
    pub path: String,
}

/// Everything one `update_agent_bundle` run changed, for the report dialog.
#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentBundleReport {
    pub agents: Vec<AgentBundleChange>,
    /// Whether the reinstall replaced `~/.claude/settings.json`. Reported
    /// because it is the user's own global Claude config — `permissions`,
    /// `env`, `hooks`, `statusLine` — and `sync_tree` overwrites it wholesale
    /// with the bundle's copy.
    pub settings_changed: bool,
    /// How many files under `~/.claude/skills` the reinstall replaced.
    pub skills_changed: u32,
    /// Where the pre-update copy of all of it now lives, for the dialog to
    /// name without hardcoding the layout core owns.
    pub backup_path: String,
}

/// Run `hyprlayer ai reinstall` and report what changed.
///
/// Backs the whole bundle up first — `agents`, `skills` and `settings.json`,
/// everything the CLI's `sync_tree` can overwrite — into a STAGING directory,
/// and promotes it to the backup of record only once the reinstall has
/// landed. That ordering is the point: the previous backup is what holds an
/// edit the previous update reverted, so destroying it up front would let a
/// reinstall that dies on a DNS error take the user's last copy with it while
/// changing nothing else. On failure the staged copy is dropped and the
/// previous one still stands.
#[tauri::command]
#[specta::specta]
pub async fn update_agent_bundle(state: State<'_, AppState>) -> CmdResult<AgentBundleReport> {
    let root = claude_root()?;

    let backup_root = root.clone();
    let staged = blocking("stage_agent_bundle_backup", move || {
        agent_bundle::stage_backup(&backup_root).map_err(AppError::from)
    })
    .await??;

    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let reinstall = HyprCli::new(&state.binaries).ai_reinstall(&cwd).await;
    if let Err(e) = reinstall {
        let discard_root = root.clone();
        let _ = blocking("discard_agent_bundle_backup", move || {
            agent_bundle::discard_staged_backup(&discard_root);
            CmdResult::Ok(())
        })
        .await;
        return Err(AppError {
            code: "agent_bundle_update_failed".into(),
            message: e.to_string(),
            remedy: Some(
                "Check your network connection and try again. The CLI's own error is under \"Technical details\"."
                    .into(),
            ),
        });
    }

    let diff_root = root.clone();
    let (diff, backup_path) = blocking("diff_agent_bundle", move || {
        let diff =
            agent_bundle::diff_against_backup(&diff_root, &staged).map_err(AppError::from)?;
        // Promoted only now: until the reinstall returned Ok there was
        // nothing worth replacing the previous backup for.
        let backup = agent_bundle::promote_backup(&diff_root).map_err(AppError::from)?;
        CmdResult::Ok((diff, backup))
    })
    .await??;

    let agents_dir = root.join("agents");
    Ok(AgentBundleReport {
        agents: diff
            .agents
            .into_iter()
            .map(|(name, change)| AgentBundleChange {
                path: agents_dir
                    .join(format!("{name}.md"))
                    .to_string_lossy()
                    .into_owned(),
                name,
                change,
            })
            .collect(),
        settings_changed: diff.settings_changed,
        skills_changed: diff.skills_changed as u32,
        backup_path: backup_path.to_string_lossy().into_owned(),
    })
}

/// Put one agent back from the rolling backup, by file stem
/// (`AgentBundleChange::name`).
#[tauri::command]
#[specta::specta]
pub async fn restore_agent_from_backup(stem: String) -> CmdResult<()> {
    let root = claude_root()?;
    blocking("restore_agent_from_backup", move || {
        agent_bundle::restore_agent_from_backup(&root, &stem).map_err(AppError::from)
    })
    .await?
}

// --- Agent bundle: version selection ---------------------------------------
//
// CLI 1.6.0 ships the bundle as a versioned GitHub release asset and can pin
// which one is installed (`ai versions`, `ai reinstall --version|--unpin`).
// The pin state itself is read through `get_ai_status`, which already carries
// it — this file adds only the list and the install.

/// How many recent releases `ai versions` examines. The CLI's own default;
/// stated here so the app's list length is a decision rather than a default
/// that could move under it.
///
/// Releases carrying no bundle are examined and dropped, so this is an upper
/// bound on the *examined* set, not on the answer.
///
/// Deliberately wider than the picker, which shows only the newest three of
/// these (`VERSION_PICKER_LIMIT` in `features/settings/bundleVersions.ts`):
/// dropped bundle-less releases must not starve the menu, and install-time
/// validation checks the choice against this window, so a displayed option
/// cannot age out of it between the menu opening and the click.
const VERSION_LIST_LIMIT: u32 = 10;

/// Refuse unless the CLI reports an installed [`MANAGEABLE_AGENT_TOOL`].
///
/// Both version-selection commands sit behind this: the backup and the
/// reinstall target `~/.claude` specifically, and `ai versions` follows the
/// CLI's *configured* harness — so on any other tool the list would offer
/// versions whose every install this app must refuse anyway, while reaching
/// GitHub for a machine the bundle surfaces otherwise leave alone. Same
/// constant, and same reasoning, as the freshness chip's gate.
async fn require_manageable(cli: &HyprCli<'_>, cwd: &Path) -> CmdResult<()> {
    let status = cli.ai_status(cwd).await.map_err(AppError::from)?;
    if !status.installed || status.agent_tool.as_deref() != Some(MANAGEABLE_AGENT_TOOL) {
        return Err(AppError {
            code: "agent_bundle_not_manageable".into(),
            message: format!("Bundle versions can only be managed for {MANAGEABLE_AGENT_TOOL}."),
            remedy: Some("Run the AI tool setup and choose Claude Code.".into()),
        });
    }
    Ok(())
}

/// `hyprlayer ai versions --json` — the only list a version picker may be
/// built from.
///
/// Gated on [`require_manageable`] like the install, not left to the caller:
/// the frontend hides the picker for a non-manageable tool, but the gate is
/// what makes offering an uninstallable list unreachable rather than merely
/// unrendered. It costs one local `ai status` spawn before the network-bound
/// call.
///
/// Network-bound, and the CLI answers from a cache at
/// `<config>/hyprlayer/cache/releases-<harness>.json` when it can — so this
/// usually still answers offline. On a cold cache with no network it exits 1
/// with `Unable to fetch the release list from GitHub: …`, which reaches the
/// UI as that message rather than an empty list. Verified against 1.6.0
/// (9a2a33c) in a network namespace.
#[tauri::command]
#[specta::specta]
pub async fn agent_bundle_versions(state: State<'_, AppState>) -> CmdResult<AiVersions> {
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let cli = HyprCli::new(&state.binaries);
    require_manageable(&cli, &cwd).await?;
    cli.ai_versions(&cwd, VERSION_LIST_LIMIT)
        .await
        .map_err(AppError::from)
}

/// The outcome of one `install_agent_bundle_version` run.
///
/// A CLI failure is data on this struct, not an `Err`: `ai reinstall` has no
/// `--json`, so its own words are all a user has to go on, and
/// `CoreError::Exec` keeps a stderr tail and drops stdout entirely. `Err` is
/// reserved for never having run the thing — an unresolvable binary, a
/// timeout, a refused precondition.
#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentBundleInstall {
    /// Whether the CLI exited 0. False still carries `stdout`/`stderr`.
    pub ok: bool,
    pub exit_code: Option<i32>,
    /// Which source actually answered. `ReleaseAsset` is the only outcome a
    /// picker selection should produce; `Fallback` means the version label
    /// now disagrees with the bytes on disk.
    pub source: BundleSource,
    pub stdout: String,
    pub stderr: String,
    /// Where the pre-install copy of the bundle now lives: the promoted
    /// backup of record on success, the kept staging copy on a CLI failure.
    /// Kept because the CLI can die mid-sync with `~/.claude` half-rewritten,
    /// and the staged copy is then the only backup holding the pre-install
    /// state — but only until the next install attempt re-stages over it,
    /// which is how the UI words it.
    pub backup_path: Option<String>,
}

/// Install a specific bundle version, or clear the pin.
///
/// `version: None` means `--unpin`: drop the pin and install the binary's own
/// bundle. The two are one command because the CLI's flags are mutually
/// exclusive — [`BundleSelection`] makes passing both unrepresentable.
///
/// `version` is checked against `ai versions` before anything is installed.
/// That is not belt-and-braces over the picker: `reinstall --version` accepts
/// *any* string and, for a version carrying no verifiable asset, silently
/// falls through to master's tree while recording the requested version in
/// the config — an install that reads as successful from both the exit code
/// and `ai status`, and that reinstalling the right version does not undo.
/// The check makes that unreachable through this command rather than merely
/// unlikely by UI convention; [`BundleSource`] is the backstop for a release
/// that disappears between the listing and the install.
///
/// Backs the bundle up first, promoting the copy to the backup of record once
/// the install returned 0 — staged like [`update_agent_bundle`], but the
/// failure paths deliberately diverge from it: the staged copy is KEPT, not
/// discarded. That flow's discard rationale assumes a failure changed nothing
/// (it names a DNS error), but a reinstall can die mid-sync — disk full, the
/// 300s timeout's SIGKILL — with `~/.claude` half-rewritten, and the staged
/// copy is then the only backup holding the pre-install state. Not *promoted*
/// on failure either: promoting after a nothing-written failure would destroy
/// the previous backup in exchange for a redundant copy of the current tree.
/// The next `stage_backup` replaces the kept copy, so it is a recovery
/// window, not a durable backup, and the payload says where it lives.
#[tauri::command]
#[specta::specta]
pub async fn install_agent_bundle_version(
    state: State<'_, AppState>,
    version: Option<String>,
) -> CmdResult<AgentBundleInstall> {
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let cli = HyprCli::new(&state.binaries);
    require_manageable(&cli, &cwd).await?;

    if let Some(v) = version.as_deref() {
        let listed = cli
            .ai_versions(&cwd, VERSION_LIST_LIMIT)
            .await
            .map_err(AppError::from)?;
        if !listed.versions.iter().any(|e| e.version == v) {
            return Err(AppError {
                code: "agent_bundle_version_unavailable".into(),
                message: format!("No agent bundle is published for version {v}."),
                remedy: Some(
                    "Pick a version from the list. Installing an unlisted version would fall back to an unversioned bundle."
                        .into(),
                ),
            });
        }
    }

    let root = claude_root()?;
    let staging_root = root.clone();
    // The staged path is held for the failure paths below: nothing here diffs
    // against the copy the way `update_agent_bundle` does, but both failure
    // shapes name where it lives.
    let staged = blocking("stage_agent_bundle_backup", move || {
        agent_bundle::stage_backup(&staging_root).map_err(AppError::from)
    })
    .await??;

    let selection = match version.as_deref() {
        Some(v) => BundleSelection::Version(v),
        None => BundleSelection::Unpin,
    };
    let output = match cli.ai_reinstall_selected(&cwd, selection).await {
        Ok(output) => output,
        // A missing binary, or the 300s timeout. The timeout SIGKILLs a CLI
        // that may already be rewriting `~/.claude`, so the staged copy stays
        // (see the function doc) and the error's remedy says where it is —
        // this arm has no payload to carry the path on.
        Err(e) => {
            let kept = format!(
                "A pre-install copy of the bundle was kept at {}.",
                staged.display()
            );
            let mut err = AppError::from(e);
            err.remedy = Some(match err.remedy {
                Some(remedy) => format!("{remedy} {kept}"),
                None => kept,
            });
            return Err(err);
        }
    };

    if output.exit_code != Some(0) {
        return Ok(AgentBundleInstall {
            ok: false,
            exit_code: output.exit_code,
            source: agent_bundle::reinstall_source(&output.stdout),
            stdout: output.stdout,
            stderr: output.stderr,
            backup_path: Some(staged.to_string_lossy().into_owned()),
        });
    }

    let backup_path = blocking("promote_agent_bundle_backup", move || {
        agent_bundle::promote_backup(&root).map_err(AppError::from)
    })
    .await??;

    Ok(AgentBundleInstall {
        ok: true,
        exit_code: output.exit_code,
        source: agent_bundle::reinstall_source(&output.stdout),
        stdout: output.stdout,
        stderr: output.stderr,
        backup_path: Some(backup_path.to_string_lossy().into_owned()),
    })
}
