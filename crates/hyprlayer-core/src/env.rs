use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct RuntimeEnvironment {
    pub home_dir: PathBuf,
    pub process_cwd: PathBuf,
    pub process_env: BTreeMap<OsString, OsString>,
}

impl RuntimeEnvironment {
    pub fn var(&self, key: &str) -> Option<&OsStr> {
        self.process_env
            .get(OsStr::new(key))
            .map(OsString::as_os_str)
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
