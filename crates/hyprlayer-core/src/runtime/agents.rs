use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::agent_bundle::{self, BundleSource, CollisionReport};
use crate::agents::{AgentContextFile, AgentFile, AgentMcpServer, AgentOverlay};
use crate::error::{CoreError, Result};
use crate::hyprcli::{AiVersions, BundleSelection, HyprCli};
use crate::skills::AgentSkill;

use super::ports::ProviderHttpRequest;
use super::HyprlayerRuntime;

const BUNDLE_PROBE_TIMEOUT: Duration = Duration::from_secs(8);
/// A ref advertisement is a few KB; the cap keeps a hostile or runaway response
/// from being buffered whole just to find one SHA in it.
const MAX_REFS_BYTES: usize = 1024 * 1024;
const VERSION_LIST_LIMIT: u32 = 10;

fn ensure_running(runtime: &HyprlayerRuntime) -> Result<()> {
    if runtime.coordinator.is_stopped() {
        return Err(CoreError::RuntimeStopped);
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_err) => Err(CoreError::Io(std::io::Error::other(join_err.to_string()))),
    }
}

async fn require_manageable(cli: &HyprCli<'_>, cwd: &Path) -> Result<()> {
    if !agent_bundle::is_manageable_pair(&cli.ai_status(cwd).await?) {
        return Err(CoreError::AgentBundleNotManageable);
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct BundleStatus {
    pub manageable: bool,
    pub installed_sha: Option<String>,
    pub latest_sha: Option<String>,
    pub installed_version: Option<String>,
    pub desired_version: Option<String>,
    pub last_checked: Option<f64>,
    pub update_available: bool,
}

#[derive(Debug, Clone)]
pub struct BundleReport {
    pub source: BundleSource,
    pub stdout: String,
}

#[derive(Debug, Clone)]
pub struct BundleInstall {
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub source: BundleSource,
    pub stdout: String,
    pub stderr: String,
    pub resolved: Option<CollisionReport>,
}

pub struct AgentService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn agents(&self) -> AgentService<'_> {
        AgentService { runtime: self }
    }
}

impl AgentService<'_> {
    fn home(&self) -> PathBuf {
        self.runtime.environment.home_dir.clone()
    }

    /// The CLI keeps the agent bundle it manages beside the config file it owns
    /// (ADR-0002), so the store root follows whichever config path the runtime
    /// was handed rather than being resolved again from the environment.
    fn store_root(&self) -> Option<PathBuf> {
        self.runtime
            .paths
            .hyprlayer_config_file
            .parent()
            .map(|dir| dir.join("agents"))
    }

    pub async fn list(&self, project_path: Option<String>) -> Result<Vec<AgentFile>> {
        ensure_running(self.runtime)?;
        let home = self.home();
        let store_root = self.store_root();
        blocking(move || {
            Ok(crate::agents::list_agents(
                project_path.as_deref().map(Path::new),
                Some(home.as_path()),
                store_root.as_deref(),
            ))
        })
        .await
    }

    pub async fn save(&self, agent: AgentFile) -> Result<AgentFile> {
        ensure_running(self.runtime)?;
        let store_root = self.store_root();
        blocking(move || crate::agents::save_agent(&agent, store_root.as_deref())).await
    }

    pub async fn delete(&self, path: String) -> Result<()> {
        ensure_running(self.runtime)?;
        let store_root = self.store_root();
        blocking(move || crate::agents::delete_agent(Path::new(&path), store_root.as_deref())).await
    }

    pub async fn new_file_path(
        &self,
        name: String,
        source: String,
        project_path: Option<String>,
    ) -> Result<Option<String>> {
        ensure_running(self.runtime)?;
        let home = self.home();
        blocking(move || {
            let dir = crate::agents::agents_dir_for(
                project_path.as_deref().map(Path::new),
                Some(home.as_path()),
                &source,
            );
            Ok(dir.map(|d| {
                d.join(format!("{}.md", crate::agents::file_stem_for(&name)))
                    .to_string_lossy()
                    .into_owned()
            }))
        })
        .await
    }

    pub async fn context_files(
        &self,
        project_path: Option<String>,
    ) -> Result<Vec<AgentContextFile>> {
        ensure_running(self.runtime)?;
        blocking(move || {
            Ok(crate::agents::context_candidates(
                project_path.as_deref().map(Path::new),
            ))
        })
        .await
    }

    pub async fn mcp_servers(&self, project_path: Option<String>) -> Result<Vec<AgentMcpServer>> {
        ensure_running(self.runtime)?;
        let home = self.home();
        blocking(move || {
            Ok(crate::agents::mcp_servers(
                project_path.as_deref().map(Path::new),
                Some(home.as_path()),
            ))
        })
        .await
    }

    pub async fn skills(&self, project_path: Option<String>) -> Result<Vec<AgentSkill>> {
        ensure_running(self.runtime)?;
        let home = self.home();
        blocking(move || {
            Ok(crate::skills::list_skills(
                project_path.as_deref().map(Path::new),
                Some(home.as_path()),
            ))
        })
        .await
    }

    pub async fn export(&self, agent: AgentFile, overlay: AgentOverlay) -> Result<String> {
        ensure_running(self.runtime)?;
        let dir = self.runtime.paths.download_dir.clone();
        blocking(move || crate::agents::export_agent(&agent, &overlay, &dir)).await
    }

    async fn fetch_latest_bundle_sha(&self) -> Option<String> {
        let request = ProviderHttpRequest::new(agent_bundle::BUNDLE_REFS_URL)
            .timeout(BUNDLE_PROBE_TIMEOUT)
            .max_bytes(MAX_REFS_BYTES);
        let response = self.runtime.provider_http.get(request).await.ok()?;
        if response.status >= 400 {
            return None;
        }
        agent_bundle::parse_master_sha(&response.body)
    }

    pub async fn bundle_status(&self) -> Result<BundleStatus> {
        ensure_running(self.runtime)?;
        let home = self.home();
        let status = HyprCli::new(&self.runtime.binaries)
            .ai_status(&home)
            .await?;

        let latest_sha = if agent_bundle::needs_sha_probe(&status) {
            self.fetch_latest_bundle_sha().await
        } else {
            None
        };
        let manageable = agent_bundle::is_manageable_pair(&status);
        let update_available = agent_bundle::update_available(&status, latest_sha.as_deref());
        let desired_version = agent_bundle::desired_version(
            status.pinned_version.as_deref(),
            status.binary_version.as_deref(),
        )
        .map(str::to_string);

        Ok(BundleStatus {
            manageable,
            installed_sha: status.agents_installed_sha,
            latest_sha,
            installed_version: status.assets_version,
            desired_version,
            last_checked: status.last_agent_check,
            update_available,
        })
    }

    pub async fn update_bundle(&self) -> Result<BundleReport> {
        ensure_running(self.runtime)?;
        let home = self.home();
        let stdout = HyprCli::new(&self.runtime.binaries)
            .ai_reinstall(&home)
            .await
            .map_err(|e| CoreError::AgentBundleUpdateFailed {
                detail: e.to_string(),
            })?;

        Ok(BundleReport {
            source: agent_bundle::reinstall_source(&stdout),
            stdout,
        })
    }

    pub async fn bundle_versions(&self) -> Result<AiVersions> {
        ensure_running(self.runtime)?;
        let home = self.home();
        let cli = HyprCli::new(&self.runtime.binaries);
        require_manageable(&cli, &home).await?;
        cli.ai_versions(&home, VERSION_LIST_LIMIT).await
    }

    pub async fn repair_bundle(&self) -> Result<BundleInstall> {
        ensure_running(self.runtime)?;
        let home = self.home();
        let cli = HyprCli::new(&self.runtime.binaries);
        let mut output = cli
            .ai_reinstall_selected(&home, BundleSelection::Force)
            .await?;

        // the CLI refuses to overwrite an occupied path, so clear what blocks it and retry once
        let mut resolved: Option<CollisionReport> = None;
        if output.exit_code != Some(0) {
            if let Ok(status) = cli.ai_status(&home).await {
                let stamp = agent_bundle::backup_stamp(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                );
                let report =
                    agent_bundle::clear_link_collisions(&status, &stamp, Some(home.as_path()));
                if !report.is_empty() {
                    output = cli
                        .ai_reinstall_selected(&home, BundleSelection::Force)
                        .await?;
                    resolved = Some(report);
                }
            }
        }

        Ok(BundleInstall {
            ok: output.exit_code == Some(0),
            exit_code: output.exit_code,
            source: agent_bundle::reinstall_source(&output.stdout),
            stdout: output.stdout,
            stderr: output.stderr,
            resolved,
        })
    }

    pub async fn install_bundle_version(&self, version: Option<String>) -> Result<BundleInstall> {
        ensure_running(self.runtime)?;
        let home = self.home();
        let cli = HyprCli::new(&self.runtime.binaries);
        require_manageable(&cli, &home).await?;

        if let Some(v) = version.as_deref() {
            let listed = cli.ai_versions(&home, VERSION_LIST_LIMIT).await?;
            if !listed.versions.iter().any(|e| e.version == v) {
                return Err(CoreError::AgentBundleVersionUnavailable {
                    version: v.to_string(),
                });
            }
        }

        let selection = match version.as_deref() {
            Some(v) => BundleSelection::Version(v),
            None => BundleSelection::Unpin,
        };
        let output = cli.ai_reinstall_selected(&home, selection).await?;

        Ok(BundleInstall {
            ok: output.exit_code == Some(0),
            exit_code: output.exit_code,
            source: agent_bundle::reinstall_source(&output.stdout),
            stdout: output.stdout,
            stderr: output.stderr,
            resolved: None,
        })
    }
}
