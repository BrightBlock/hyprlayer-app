use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::process::binaries::{BinaryKind, BinaryResolver};
use crate::process::{Exec, ExecOutput};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct StorageSettings {
    pub thoughts_repo: Option<String>,
    pub repos_dir: Option<String>,
    pub global_dir: Option<String>,
    pub vault_path: Option<String>,
    pub vault_subpath: Option<String>,
    pub content_root: Option<String>,
    pub parent_page_id: Option<String>,
    pub database_id: Option<String>,
    pub space_id: Option<String>,
    pub type_id: Option<String>,
    pub api_token_env: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SchemaFieldInfo {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    pub required: bool,
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct AiStatus {
    pub agent_tool: Option<String>,
    pub installed: bool,
    pub location: Option<String>,
    pub agents_installed_sha: Option<String>,
    pub last_agent_check: Option<f64>,
    pub assets_version: Option<String>,
    pub binary_version: Option<String>,
    pub pinned_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AiVersionEntry {
    pub version: String,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub current: bool,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct AiVersions {
    pub assets_version: Option<String>,
    pub binary_version: Option<String>,
    pub harness: Option<String>,
    pub pinned_version: Option<String>,
    pub versions: Vec<AiVersionEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct CliUpdateCheck {
    pub current: Option<String>,
    pub latest: Option<String>,
    pub hint: Option<String>,
}

pub fn parse_self_update_check(stdout: &str) -> CliUpdateCheck {
    for line in stdout.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Update available: ") {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleSelection<'a> {
    Version(&'a str),
    Unpin,
}

pub struct HyprCli<'a> {
    resolver: &'a BinaryResolver,
}

impl<'a> HyprCli<'a> {
    pub fn new(resolver: &'a BinaryResolver) -> Self {
        Self { resolver }
    }

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

    pub async fn thoughts_sync(&self, repo_path: &Path, message: Option<&str>) -> Result<String> {
        let mut args = vec!["thoughts", "sync"];
        if let Some(m) = message {
            args.push("-m");
            args.push(m);
        }
        self.exec(repo_path, &args, Duration::from_secs(300)).await
    }

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

    pub async fn thoughts_uninit(&self, repo_path: &Path) -> Result<String> {
        self.exec(repo_path, &["thoughts", "uninit"], Duration::from_secs(60))
            .await
    }

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

    pub async fn ai_status(&self, cwd: &Path) -> Result<AiStatus> {
        let stdout = self
            .exec(cwd, &["ai", "status", "--json"], Duration::from_secs(30))
            .await?;
        serde_json::from_str(&stdout).map_err(|e| CoreError::OutputParse {
            program: "hyprlayer ai status".into(),
            detail: e.to_string(),
        })
    }

    pub async fn ai_reinstall(&self, cwd: &Path) -> Result<String> {
        self.exec(cwd, &["ai", "reinstall"], Duration::from_secs(300))
            .await
    }

    pub async fn ai_versions(&self, cwd: &Path, limit: u32) -> Result<AiVersions> {
        let limit = limit.to_string();
        let stdout = self
            .exec(
                cwd,
                &["ai", "versions", "--json", "--limit", &limit],
                Duration::from_secs(30),
            )
            .await?;
        serde_json::from_str(&stdout).map_err(|e| CoreError::OutputParse {
            program: "hyprlayer ai versions".into(),
            detail: e.to_string(),
        })
    }

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

    #[test]
    fn reads_a_null_pin_as_unpinned() {
        let s: AiStatus = serde_json::from_str(
            r#"{"agentTool":"Claude Code","installed":true,"assetsVersion":"1.6.0","binaryVersion":"1.6.0","pinnedVersion":null}"#,
        )
        .unwrap();
        assert_eq!(s.pinned_version, None);
        assert_eq!(s.assets_version.as_deref(), Some("1.6.0"));
    }

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
        assert_eq!(v.versions[2].version, "1.6.0-rc.1");
        assert!(!v.versions[2].current && !v.versions[2].pinned);
    }

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
