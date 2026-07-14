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
use crate::process::Exec;

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
/// The CLI emits additional fields (location, version, …) that we don't need
/// yet; they are captured by `#[serde(default)]` and ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct AiStatus {
    /// The agent tool name as reported by the CLI, e.g. `"Claude Code"`.
    pub agent_tool: Option<String>,
    /// Whether the agent tool is installed and ready to use.
    pub installed: bool,
    /// Filesystem path to the agent binary, when installed.
    pub location: Option<String>,
}

pub struct HyprCli<'a> {
    resolver: &'a BinaryResolver,
}

impl<'a> HyprCli<'a> {
    pub fn new(resolver: &'a BinaryResolver) -> Self {
        Self { resolver }
    }

    async fn exec(&self, cwd: &Path, args: &[&str], timeout: Duration) -> Result<String> {
        let bin = self.resolver.require(BinaryKind::Hyprlayer).await?;
        let path = self.resolver.augmented_path().await;
        let output = Exec::new(bin)
            .args(args.iter().copied())
            .cwd(cwd)
            .env("PATH", path)
            .timeout(timeout)
            .run()
            .await?;
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
    /// (the config is global) — any existing `cwd` works.
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

    /// `hyprlayer ai status --json` — typed; only the three fields the
    /// onboarding step needs; extra CLI fields are silently ignored.
    pub async fn ai_status(&self, cwd: &Path) -> Result<AiStatus> {
        let stdout = self
            .exec(cwd, &["ai", "status", "--json"], Duration::from_secs(30))
            .await?;
        serde_json::from_str(&stdout).map_err(|e| CoreError::OutputParse {
            program: "hyprlayer ai status".into(),
            detail: e.to_string(),
        })
    }
}
