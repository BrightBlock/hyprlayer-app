use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};
use tempfile::TempDir;

/// The resolver searches well-known install dirs whatever `PATH` says, so
/// providers and `gh` are pinned to a missing file to keep the network out.
const ABSENT: [&str; 5] = ["claude", "codex", "opencode", "gh", "ollama"];

/// Fixed so fixture commits get the same SHA on every run.
const COMMIT_DATE: &str = "2026-01-01T00:00:00Z";

const GITCONFIG: &str = "[user]\n\tname = Hyprlayer E2E\n\temail = e2e@hyprlayer.invalid\n\
[commit]\n\tgpgsign = false\n[tag]\n\tgpgsign = false\n[init]\n\tdefaultBranch = main\n";

/// A CLI that is already installed and opted out: without the full telemetry
/// block the CLI re-enrolls a fresh install in anonymous telemetry, and without
/// `disableUpdateCheck` it asks GitHub for a newer release.
const CLI_CONFIG: &str = r#"{
  "version": 4,
  "disableUpdateCheck": true,
  "autoUpdate": false,
  "telemetry": {
    "mode": "off",
    "installationId": "00000000-0000-4000-8000-000000000e2e",
    "deviceSalt": "00000000000000000000000000000e2e",
    "apiKeySource": "default",
    "lastFlush": 0,
    "lastConfigRefresh": 0,
    "lastEnrollmentCheck": 0
  }
}
"#;

/// A throwaway profile: its own `HOME`, app dirs, logs and fixture repos.
pub struct Sandbox {
    dir: Option<TempDir>,
    /// As the OS handed it out, before symlinks resolve (`/var` on macOS).
    pub given: PathBuf,
    pub root: PathBuf,
    pub home: PathBuf,
    pub app_config: PathBuf,
    pub app_data: PathBuf,
    pub logs: PathBuf,
    /// The server's working directory: a git repository with nothing in it,
    /// so input resolved against the daemon's cwd shows up rather than failing.
    pub cwd: PathBuf,
    pub git: PathBuf,
    pub hyprlayer: PathBuf,
    bin: PathBuf,
    tmp: PathBuf,
    run: PathBuf,
}

impl Sandbox {
    pub fn new() -> Self {
        let git = find_tool("git");
        let hyprlayer = find_tool("hyprlayer");
        let dir = tempfile::Builder::new()
            .prefix("hl-e2e-")
            .tempdir()
            .expect("create the sandbox dir");
        let given = dir.path().to_path_buf();
        let root = std::fs::canonicalize(&given).expect("canonicalize the sandbox");
        let sandbox = Self {
            home: root.join("home"),
            app_config: root.join("app-config"),
            app_data: root.join("app-data"),
            logs: root.join("logs"),
            cwd: root.join("daemon-cwd"),
            bin: root.join("bin"),
            tmp: root.join("tmp"),
            run: root.join("run"),
            dir: Some(dir),
            given,
            root,
            git,
            hyprlayer,
        };
        for dir in [
            &sandbox.home,
            &sandbox.app_config,
            &sandbox.app_data,
            &sandbox.logs,
            &sandbox.cwd,
            &sandbox.bin,
            &sandbox.tmp,
            &sandbox.run,
        ] {
            std::fs::create_dir_all(dir).expect("create a sandbox dir");
        }
        // Only these two on PATH: the CLI's own lookups would otherwise find
        // gh and the providers beside hyprlayer in /opt/homebrew/bin.
        #[cfg(unix)]
        for (name, tool) in [("git", &sandbox.git), ("hyprlayer", &sandbox.hyprlayer)] {
            std::os::unix::fs::symlink(tool, sandbox.bin.join(name)).expect("link a tool");
        }
        let cli_config = sandbox.cli_config();
        std::fs::create_dir_all(cli_config.parent().expect("CLI config dir"))
            .expect("create the CLI config dir");
        std::fs::write(&cli_config, CLI_CONFIG).expect("seed the CLI config");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&sandbox.run, std::fs::Permissions::from_mode(0o700))
                .expect("narrow the runtime dir");
        }
        std::fs::write(sandbox.home.join(".gitconfig"), GITCONFIG).expect("write .gitconfig");
        sandbox.write_settings();
        sandbox.git(
            &sandbox.cwd,
            &["init", "--quiet", "--initial-branch", "main"],
        );
        sandbox
    }

    fn write_settings(&self) {
        let mut pins = serde_json::Map::new();
        pins.insert("git".into(), json!(self.git));
        pins.insert("hyprlayer".into(), json!(self.hyprlayer));
        for name in ABSENT {
            pins.insert(name.into(), json!(self.root.join("absent").join(name)));
        }
        let settings = json!({ "binaryOverrides": pins });
        std::fs::write(
            self.app_config.join("settings.json"),
            serde_json::to_vec_pretty(&settings).expect("encode settings"),
        )
        .expect("write settings.json");
    }

    /// The same path under the uncanonicalized root, to prove the server
    /// canonicalizes what it registers.
    pub fn as_given(&self, path: &Path) -> PathBuf {
        match path.strip_prefix(&self.root) {
            Ok(rest) => self.given.join(rest),
            Err(_) => path.to_path_buf(),
        }
    }

    /// The hyprlayer CLI's config, which it resolves from `HOME`.
    pub fn cli_config(&self) -> PathBuf {
        let base = if cfg!(target_os = "macos") {
            self.home.join("Library").join("Application Support")
        } else {
            self.home.join(".config")
        };
        base.join("hyprlayer").join("config.json")
    }

    /// Where the server binds: `$XDG_RUNTIME_DIR` on Linux, else the data dir.
    pub fn socket(&self) -> PathBuf {
        if cfg!(target_os = "linux") {
            let hash = hyprlayer_server::socket::profile_hash(&self.app_data);
            self.run.join("hyprlayer").join(format!("{hash}.sock"))
        } else {
            self.app_data.join("hyprlayer.sock")
        }
    }

    /// The server's whole environment. Nothing is inherited: the shell may
    /// export the release profile's `HYPRLAYER_APP_DATA_DIR`.
    pub fn server_env(&self) -> Vec<(&'static str, OsString)> {
        vec![
            ("HOME", self.home.clone().into()),
            ("USER", "e2e".into()),
            ("LOGNAME", "e2e".into()),
            ("PATH", self.path_var()),
            ("SHELL", "/bin/sh".into()),
            ("TMPDIR", self.tmp.clone().into()),
            ("XDG_RUNTIME_DIR", self.run.clone().into()),
            ("GIT_CONFIG_NOSYSTEM", "1".into()),
            ("HYPRLAYER_APP_CONFIG_DIR", self.app_config.clone().into()),
            ("HYPRLAYER_APP_DATA_DIR", self.app_data.clone().into()),
            ("HYPRLAYER_APP_LOG_DIR", self.logs.clone().into()),
            ("HYPRLAYER_LOG", "debug".into()),
        ]
    }

    fn path_var(&self) -> OsString {
        let dirs = [
            self.bin.clone(),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
            PathBuf::from("/usr/sbin"),
            PathBuf::from("/sbin"),
        ];
        std::env::join_paths(dirs).expect("join PATH")
    }

    /// Runs git with the server's environment and fixed commit dates; fails
    /// the test on a non-zero exit.
    pub fn git(&self, dir: &Path, args: &[&str]) -> String {
        let output = Command::new(&self.git)
            .arg("-C")
            .arg(dir)
            .args(args)
            .env_clear()
            .envs(self.server_env())
            .env("GIT_AUTHOR_DATE", COMMIT_DATE)
            .env("GIT_COMMITTER_DATE", COMMIT_DATE)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} in {} failed: {}",
            dir.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// A committed repository at `~/code/<name>` on `main`.
    pub fn fixture_repo(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let repo = self.home.join("code").join(name);
        std::fs::create_dir_all(&repo).expect("create the fixture repo");
        self.git(&repo, &["init", "--quiet", "--initial-branch", "main"]);
        for (path, content) in files {
            let file = repo.join(path);
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).expect("create a fixture dir");
            }
            std::fs::write(file, content).expect("write a fixture file");
        }
        self.git(&repo, &["add", "--all"]);
        self.git(&repo, &["commit", "--quiet", "--message", "Initial commit"]);
        repo
    }

    pub fn branch_exists(&self, repo: &Path, branch: &str) -> bool {
        !self
            .git(repo, &["branch", "--list", branch])
            .trim()
            .is_empty()
    }

    /// The paths `git worktree list` reports for `repo`.
    pub fn worktrees(&self, repo: &Path) -> Vec<String> {
        self.git(repo, &["worktree", "list", "--porcelain"])
            .lines()
            .filter_map(|line| line.strip_prefix("worktree "))
            .map(str::to_string)
            .collect()
    }

    /// Each entry of `dir` with its symlink target, if any, sorted.
    pub fn listing(&self, dir: &Path) -> Value {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Value::Null;
        };
        let mut listed: Vec<(String, Value)> = entries
            .flatten()
            .map(|entry| {
                let target = std::fs::read_link(entry.path())
                    .map(|t| json!(t))
                    .unwrap_or(Value::Null);
                (entry.file_name().to_string_lossy().into_owned(), target)
            })
            .collect();
        listed.sort_by(|a, b| a.0.cmp(&b.0));
        Value::Object(listed.into_iter().collect())
    }

    /// Everything the server has logged so far, across its daily files.
    pub fn server_log(&self) -> String {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.logs)
            .map(|entries| entries.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        files.retain(|f| {
            f.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("hyprlayer-server."))
        });
        files.sort();
        files.iter().filter_map(|f| self.read(f)).collect()
    }

    pub fn read(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    pub fn read_json(&self, path: &Path) -> Value {
        self.read(path)
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or(Value::Null)
    }

    /// `git --version` and `hyprlayer --version`, for `environment.json`.
    pub fn tool_versions(&self) -> Value {
        let version = |tool: &Path| {
            Command::new(tool)
                .arg("--version")
                .env_clear()
                .envs(self.server_env())
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default()
        };
        json!({ "git": version(&self.git), "hyprlayer": version(&self.hyprlayer) })
    }
}

impl Default for Sandbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        if std::env::var_os("HYPRLAYER_E2E_KEEP").is_some() {
            if let Some(dir) = self.dir.take() {
                eprintln!("keeping the sandbox at {}", dir.keep().display());
            }
        }
    }
}

/// `name` on this process's `PATH`, then the install dirs the server's own
/// resolver searches. Missing prerequisites fail loudly rather than skip.
fn find_tool(name: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    if let Some(home) = home {
        dirs.extend([home.join(".cargo/bin"), home.join(".local/bin")]);
    }
    dirs.into_iter()
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| {
            panic!("the E2E harness needs `{name}` installed; it was not found on PATH")
        })
}
