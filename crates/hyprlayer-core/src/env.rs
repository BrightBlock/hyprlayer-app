use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::{Arc, PoisonError, RwLock};

#[derive(Debug, Clone)]
pub struct RuntimeEnvironment {
    pub home_dir: PathBuf,
    pub process_cwd: PathBuf,
    pub process_env: BTreeMap<OsString, OsString>,
    /// Per-attach overrides merged over `process_env` at child spawn — the one
    /// mutable exception to freeze-at-construction (ADR-0009). Unread until then.
    pub env_overlay: Arc<RwLock<BTreeMap<String, String>>>,
}

impl RuntimeEnvironment {
    /// The embedding process's own environment. Called by an embedder at
    /// construction, never by a core operation.
    pub fn detect() -> Self {
        Self {
            home_dir: dirs::home_dir().unwrap_or_else(|| {
                tracing::warn!("no home directory in the environment; using the working directory");
                PathBuf::from(".")
            }),
            process_cwd: std::env::current_dir().unwrap_or_else(|e| {
                tracing::warn!("no working directory ({e}); using \".\"");
                PathBuf::from(".")
            }),
            process_env: std::env::vars_os().collect(),
            env_overlay: Arc::default(),
        }
    }

    pub fn with_env_overlay(self, overlay: BTreeMap<String, String>) -> Self {
        self.merge_env_overlay(overlay);
        self
    }

    /// Merges an attach's keys over the live overlay; a key is never cleared (ADR-0009).
    pub fn merge_env_overlay(&self, overlay: BTreeMap<String, String>) {
        self.env_overlay
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(overlay);
    }

    /// A snapshot of the overlay, for merging over a spawned child's environment.
    pub fn overlay_env(&self) -> Vec<(String, String)> {
        self.env_overlay
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Windows variable names are case-insensitive (`PATH` is stored as `Path`),
    /// so an exact miss falls back to a case-insensitive match there.
    pub fn var(&self, key: &str) -> Option<&OsStr> {
        if let Some(value) = self.process_env.get(OsStr::new(key)) {
            return Some(value.as_os_str());
        }
        if cfg!(windows) {
            return self
                .process_env
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.as_os_str());
        }
        None
    }
}

#[derive(Debug, Clone)]
pub struct RuntimePaths {
    pub hyprlayer_config_file: PathBuf,
    pub app_config_dir: PathBuf,
    pub app_data_dir: PathBuf,
    pub default_worktree_root: PathBuf,
    pub default_repos_root: PathBuf,
    pub opencode_config_file: PathBuf,
    pub opencode_auth_file: PathBuf,
    pub opencode_data_dir: PathBuf,
    pub claude_config_dir: PathBuf,
    pub claude_managed_settings_files: Vec<PathBuf>,
    pub codex_home: PathBuf,
    pub agent_roots: Vec<PathBuf>,
    pub download_dir: PathBuf,
}

impl RuntimePaths {
    /// The desktop's 0.7.5 resolution, minus the two Tauri-derived app dirs the
    /// embedder must supply. The one place core reads `dirs::` for these paths.
    pub fn detect(
        env: &RuntimeEnvironment,
        app_config_dir: PathBuf,
        app_data_dir: PathBuf,
    ) -> Self {
        let home = &env.home_dir;
        Self {
            hyprlayer_config_file: hyprlayer_config_file(),
            app_config_dir,
            app_data_dir,
            default_worktree_root: home.join("hyprlayer").join("worktrees"),
            default_repos_root: home.join("hyprlayer").join("repos"),
            opencode_config_file: opencode_config_file(env),
            opencode_auth_file: opencode_auth_file(env),
            opencode_data_dir: dirs::data_dir()
                .unwrap_or_else(|| home.join(".local").join("share"))
                .join("opencode"),
            claude_config_dir: claude_config_dir(env),
            claude_managed_settings_files: claude_managed_settings_files(),
            codex_home: env
                .var("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex")),
            agent_roots: vec![home.join(".claude").join("agents")],
            download_dir: dirs::download_dir().unwrap_or_else(|| home.clone()),
        }
    }
}

fn hyprlayer_config_file() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| {
            tracing::warn!(
                "no config directory in the environment; looking for hyprlayer's config in \".\""
            );
            PathBuf::from(".")
        })
        .join("hyprlayer")
        .join("config.json")
}

pub fn opencode_auth_file(env: &RuntimeEnvironment) -> PathBuf {
    if let Some(p) = env.var("OPENCODE_AUTH_JSON") {
        return PathBuf::from(p);
    }
    env.var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| env.home_dir.join(".local").join("share"))
        .join("opencode")
        .join("auth.json")
}

pub fn opencode_config_file(env: &RuntimeEnvironment) -> PathBuf {
    if let Some(p) = env.var("OPENCODE_CONFIG") {
        return PathBuf::from(p);
    }
    env.var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| env.home_dir.join(".config"))
        .join("opencode")
        .join("opencode.json")
}

fn claude_config_dir(env: &RuntimeEnvironment) -> PathBuf {
    env.var("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| env.home_dir.join(".claude"))
}

/// The rule every URL test knob shares: `https://`, or `http://` to a loopback host.
pub(crate) fn https_or_loopback_http(raw: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(raw) else {
        return false;
    };
    match url.scheme() {
        "https" => true,
        "http" => url.host_str().is_some_and(|host| {
            let host = host.trim_start_matches('[').trim_end_matches(']');
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        }),
        _ => false,
    }
}

fn claude_managed_settings_files() -> Vec<PathBuf> {
    vec![PathBuf::from(if cfg!(target_os = "windows") {
        r"C:\Program Files\ClaudeCode\managed-settings.json"
    } else if cfg!(target_os = "macos") {
        "/Library/Application Support/ClaudeCode/managed-settings.json"
    } else {
        "/etc/claude-code/managed-settings.json"
    })]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(home: &str, vars: &[(&str, &str)]) -> RuntimeEnvironment {
        RuntimeEnvironment {
            home_dir: PathBuf::from(home),
            process_cwd: PathBuf::from(home),
            process_env: vars
                .iter()
                .map(|(k, v)| (OsString::from(k), OsString::from(v)))
                .collect(),
            env_overlay: Arc::default(),
        }
    }

    #[test]
    fn var_matches_windows_mixed_case_names() {
        let git = OsStr::new(r"C:\Program Files\Git\cmd");
        let env = environment("/tmp/hl-home", &[("Path", r"C:\Program Files\Git\cmd")]);
        assert_eq!(env.var("Path"), Some(git));
        assert_eq!(env.var("PATH"), cfg!(windows).then_some(git));
    }

    // The oracle is the desktop's 0.7.5 `resolve_paths`, field by field.
    #[test]
    fn detect_reproduces_the_desktop_paths_for_a_given_home() {
        let home = PathBuf::from("/tmp/hl-home");
        let env = environment("/tmp/hl-home", &[]);
        let paths = RuntimePaths::detect(
            &env,
            PathBuf::from("/tmp/hl-app-config"),
            PathBuf::from("/tmp/hl-app-data"),
        );

        assert_eq!(
            paths.hyprlayer_config_file,
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("hyprlayer")
                .join("config.json")
        );
        assert_eq!(paths.app_config_dir, PathBuf::from("/tmp/hl-app-config"));
        assert_eq!(paths.app_data_dir, PathBuf::from("/tmp/hl-app-data"));
        assert_eq!(
            paths.default_worktree_root,
            home.join("hyprlayer").join("worktrees")
        );
        assert_eq!(
            paths.default_repos_root,
            home.join("hyprlayer").join("repos")
        );
        assert_eq!(
            paths.opencode_config_file,
            home.join(".config").join("opencode").join("opencode.json")
        );
        assert_eq!(
            paths.opencode_auth_file,
            home.join(".local")
                .join("share")
                .join("opencode")
                .join("auth.json")
        );
        assert_eq!(
            paths.opencode_data_dir,
            dirs::data_dir()
                .unwrap_or_else(|| home.join(".local").join("share"))
                .join("opencode")
        );
        assert_eq!(paths.claude_config_dir, home.join(".claude"));
        assert_eq!(
            paths.claude_managed_settings_files,
            vec![PathBuf::from(if cfg!(target_os = "windows") {
                r"C:\Program Files\ClaudeCode\managed-settings.json"
            } else if cfg!(target_os = "macos") {
                "/Library/Application Support/ClaudeCode/managed-settings.json"
            } else {
                "/etc/claude-code/managed-settings.json"
            })]
        );
        assert_eq!(paths.codex_home, home.join(".codex"));
        assert_eq!(paths.agent_roots, vec![home.join(".claude").join("agents")]);
        assert_eq!(
            paths.download_dir,
            dirs::download_dir().unwrap_or_else(|| home.clone())
        );
    }

    #[test]
    fn detect_honours_the_explicit_override_variables() {
        let env = environment(
            "/tmp/hl-home",
            &[
                ("OPENCODE_AUTH_JSON", "/etc/oc/auth.json"),
                ("OPENCODE_CONFIG", "/etc/oc/opencode.json"),
                ("CLAUDE_CONFIG_DIR", "/etc/claude"),
                ("CODEX_HOME", "/etc/codex"),
            ],
        );
        let paths = RuntimePaths::detect(&env, PathBuf::from("/c"), PathBuf::from("/d"));
        assert_eq!(paths.opencode_auth_file, PathBuf::from("/etc/oc/auth.json"));
        assert_eq!(
            paths.opencode_config_file,
            PathBuf::from("/etc/oc/opencode.json")
        );
        assert_eq!(paths.claude_config_dir, PathBuf::from("/etc/claude"));
        assert_eq!(paths.codex_home, PathBuf::from("/etc/codex"));

        let xdg = environment(
            "/tmp/hl-home",
            &[
                ("XDG_DATA_HOME", "/tmp/xdg-data"),
                ("XDG_CONFIG_HOME", "/tmp/xdg-config"),
            ],
        );
        let paths = RuntimePaths::detect(&xdg, PathBuf::from("/c"), PathBuf::from("/d"));
        assert_eq!(
            paths.opencode_auth_file,
            PathBuf::from("/tmp/xdg-data/opencode/auth.json")
        );
        assert_eq!(
            paths.opencode_config_file,
            PathBuf::from("/tmp/xdg-config/opencode/opencode.json")
        );
    }

    #[test]
    fn env_overlay_is_declared_and_settable() {
        let env = environment("/tmp/hl-home", &[]).with_env_overlay(BTreeMap::from([(
            "SSH_AUTH_SOCK".into(),
            "/tmp/probe".into(),
        )]));
        let overlay = env.env_overlay.read().unwrap();
        assert_eq!(
            overlay.get("SSH_AUTH_SOCK").map(String::as_str),
            Some("/tmp/probe")
        );
    }
}
