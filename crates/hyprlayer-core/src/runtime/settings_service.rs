use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_models::RuntimeEvent;
use serde::{Deserialize, Serialize};

use crate::config::EffectiveConfig;
use crate::error::{CoreError, Result};
use crate::hyprcli::{CliUpdateCheck, HyprCli};
use crate::pr;
use crate::process::binaries::{BinaryKind, BinaryStatus};
use crate::process::Exec;
use crate::runtime::terminals::{default_shell, login_shell_args};
use crate::settings::AppSettings;

use super::HyprlayerRuntime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum CliInstallMethodId {
    Homebrew,
    Winget,
    Scoop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CliInstallMethod {
    pub id: CliInstallMethodId,
    pub label: String,
    pub command: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CliUpdateResult {
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

struct CliCandidate {
    id: CliInstallMethodId,
    label: &'static str,
    command: &'static str,
    probe: &'static str,
}

fn cli_install_candidates() -> &'static [CliCandidate] {
    #[cfg(not(target_os = "windows"))]
    let candidates: &[CliCandidate] = &[CliCandidate {
        id: CliInstallMethodId::Homebrew,
        label: "Homebrew",
        command: "brew install brightblock/tap/hyprlayer",
        probe: "brew",
    }];
    #[cfg(target_os = "windows")]
    let candidates: &[CliCandidate] = &[
        CliCandidate {
            id: CliInstallMethodId::Winget,
            label: "winget",
            command: "winget install BrightBlock.Hyprlayer --force --accept-source-agreements --accept-package-agreements",
            probe: "winget",
        },
        CliCandidate {
            id: CliInstallMethodId::Scoop,
            label: "Scoop",
            command: "scoop install hyprlayer",
            probe: "scoop",
        },
    ];
    candidates
}

async fn package_manager_available(shell: &Path, path: &std::ffi::OsStr, manager: &str) -> bool {
    Exec::new(shell)
        .args(login_shell_args(&format!("{manager} --version")))
        .env("PATH", path)
        .timeout(Duration::from_secs(15))
        .run()
        .await
        .is_ok()
}

pub struct SettingsService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn settings(&self) -> SettingsService<'_> {
        SettingsService { runtime: self }
    }
}

impl SettingsService<'_> {
    pub async fn get(&self) -> AppSettings {
        self.runtime.settings.get().await
    }

    pub async fn set(&self, settings: AppSettings) -> Result<()> {
        self.runtime.settings.set(settings).await
    }

    pub async fn update<F>(&self, mutate: F) -> Result<AppSettings>
    where
        F: FnOnce(&mut AppSettings),
    {
        self.runtime.settings.update(mutate).await
    }

    pub async fn effective_config_for(&self, project_path: &str) -> Result<EffectiveConfig> {
        let loaded = self.runtime.config.load().await?;
        let thoughts = loaded
            .config
            .thoughts
            .as_ref()
            .ok_or(CoreError::ThoughtsNotConfigured)?;
        Ok(thoughts.effective_config_for(project_path))
    }

    pub async fn set_binary_override(
        &self,
        kind: BinaryKind,
        path: Option<String>,
    ) -> Result<Vec<BinaryStatus>> {
        self.runtime
            .settings
            .update(|s| match &path {
                Some(p) => {
                    s.binary_overrides.insert(kind, p.clone());
                }
                None => {
                    s.binary_overrides.remove(&kind);
                }
            })
            .await?;

        self.runtime
            .binaries
            .set_override(kind, path.map(PathBuf::from))
            .await;

        let statuses = self.runtime.binaries.status_all().await;
        let _ = self.runtime.events.send(RuntimeEvent::BinariesChanged);
        Ok(statuses)
    }

    pub async fn redetect_binaries(&self) -> Vec<BinaryStatus> {
        self.runtime.binaries.redetect().await;
        let statuses = self.runtime.binaries.status_all().await;
        let _ = self.runtime.events.send(RuntimeEvent::BinariesChanged);
        if self.runtime.migrate_config_if_needed().await {
            let _ = self.runtime.events.send(RuntimeEvent::ConfigChanged);
        }
        statuses
    }

    pub async fn github_username(&self) -> Option<String> {
        let Ok(gh) = self.runtime.binaries.require(BinaryKind::Gh).await else {
            return None;
        };
        pr::github_login(&gh, &self.runtime.environment.overlay_env()).await
    }

    /// The username a branch created here is attributed to: this machine's
    /// own `USER`, not whichever client happens to be attached, because this
    /// process's git makes the commit (ADR-0009, plan Phase 5).
    pub async fn default_username(&self) -> String {
        self.runtime
            .environment
            .var("USER")
            .or_else(|| self.runtime.environment.var("USERNAME"))
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_else(|| "user".to_string())
    }

    pub async fn detect_cli_install_methods(&self) -> Vec<CliInstallMethod> {
        let path = self.runtime.binaries.augmented_path().await;
        let shell = default_shell();

        let mut methods = Vec::new();
        for c in cli_install_candidates() {
            methods.push(CliInstallMethod {
                id: c.id,
                label: c.label.to_string(),
                command: c.command.to_string(),
                available: package_manager_available(&shell, &path, c.probe).await,
            });
        }
        methods
    }

    pub async fn install_hyprlayer_cli(&self, method: CliInstallMethodId) -> Result<()> {
        let command = cli_install_candidates()
            .iter()
            .find(|c| c.id == method)
            .map(|c| c.command)
            .ok_or(CoreError::CliInstallUnsupported)?;

        let path = self.runtime.binaries.augmented_path().await;
        let shell = default_shell();

        Exec::new(&shell)
            .args(login_shell_args(command))
            .env("PATH", path)
            .timeout(Duration::from_secs(300))
            .run()
            .await?;

        self.runtime.binaries.redetect().await;
        let _ = self.runtime.events.send(RuntimeEvent::BinariesChanged);
        Ok(())
    }

    pub async fn hyprlayer_cli_update_check(&self) -> Result<CliUpdateCheck> {
        HyprCli::new(&self.runtime.binaries)
            .self_update_check(&self.runtime.environment.home_dir)
            .await
    }

    pub async fn update_hyprlayer_cli(&self) -> Result<CliUpdateResult> {
        let output = HyprCli::new(&self.runtime.binaries)
            .self_update(&self.runtime.environment.home_dir)
            .await?;

        self.runtime.binaries.redetect().await;
        let _ = self.runtime.events.send(RuntimeEvent::BinariesChanged);

        Ok(CliUpdateResult {
            ok: output.exit_code == Some(0),
            exit_code: output.exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    pub async fn install_opencode(&self) -> Result<()> {
        let path = self.runtime.binaries.augmented_path().await;
        let shell = default_shell();

        #[cfg(windows)]
        {
            let has_npm = Exec::new(&shell)
                .args([
                    "-NoProfile".to_string(),
                    "-Command".to_string(),
                    "npm --version".to_string(),
                ])
                .env("PATH", path.as_os_str())
                .timeout(Duration::from_secs(20))
                .run()
                .await
                .is_ok();
            if !has_npm {
                return Err(CoreError::NpmMissing);
            }
        }

        #[cfg(not(windows))]
        let args = vec![
            "-lc".to_string(),
            "curl -fsSL https://opencode.ai/install | bash".to_string(),
        ];
        #[cfg(windows)]
        let args = vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            "npm install -g opencode-ai".to_string(),
        ];

        let result = Exec::new(&shell)
            .args(args)
            .env("PATH", path)
            .timeout(Duration::from_secs(300))
            .run()
            .await;

        #[cfg(windows)]
        result.map_err(|e| CoreError::OpencodeInstallFailed {
            detail: e.to_string(),
        })?;
        #[cfg(not(windows))]
        result?;

        self.runtime.binaries.redetect().await;
        let _ = self.runtime.events.send(RuntimeEvent::BinariesChanged);
        Ok(())
    }
}
