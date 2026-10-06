use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{CoreError, Result};
use crate::process::Exec;

use super::SkillSourceRef;

pub const MAX_FILES: usize = 500;
pub const MAX_BYTES: u64 = 20 * 1024 * 1024;
const LS_REMOTE_TIMEOUT: Duration = Duration::from_secs(30);
const FETCH_TIMEOUT: Duration = Duration::from_secs(120);
const CHECKOUT_TIMEOUT: Duration = Duration::from_secs(60);
const LOCAL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_HINT_CANDIDATES: usize = 50;

#[derive(Debug, Clone)]
pub struct Resolved {
    pub git_ref: String,
    pub commit: String,
    /// The skill path inside the repo; `None` means discover it.
    pub path: Option<String>,
    /// With no path, the skill folder (or frontmatter `name`) to pick.
    pub name_hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TreeEntry {
    /// Relative to the skill path.
    pub rel_path: String,
    pub executable: bool,
    pub bytes: u64,
}

pub struct Fetched {
    pub repo_dir: PathBuf,
    pub path: String,
    pub entries: Vec<TreeEntry>,
}

/// Every git run: no prompts, no user hooks, bytes exactly as committed.
pub struct Git<'a> {
    pub bin: &'a Path,
    pub overlay: &'a [(String, String)],
    pub staging: &'a Path,
}

impl Git<'_> {
    fn exec(&self, dir: &Path, args: &[&str], timeout: Duration) -> Exec {
        let hooks = format!("core.hooksPath={}", self.staging.join("no-hooks").display());
        Exec::new(self.bin)
            .arg("-C")
            .arg(dir)
            .args(["--literal-pathspecs", "-c"])
            .arg(hooks)
            .args([
                "-c",
                "credential.interactive=never",
                "-c",
                "core.autocrlf=false",
                "-c",
                "core.eol=lf",
                "-c",
                "filter.lfs.smudge=",
                "-c",
                "filter.lfs.process=",
                "-c",
                "filter.lfs.required=false",
            ])
            .args(args)
            .envs(self.overlay.iter().cloned())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .env("GIT_LFS_SKIP_SMUDGE", "1")
            .env("GIT_ASKPASS", "")
            .env("SSH_ASKPASS", "")
            .timeout(timeout)
    }

    async fn run(&self, dir: &Path, args: &[&str], timeout: Duration) -> Result<String> {
        match self.exec(dir, args, timeout).run().await {
            Ok(out) => Ok(out.stdout),
            Err(CoreError::Exec { stderr_tail, .. }) => Err(CoreError::SkillFetchFailed {
                detail: stderr_tail,
            }),
            Err(CoreError::ExecTimeout { seconds, .. }) => Err(CoreError::SkillFetchFailed {
                detail: format!("Timed out after {seconds}s."),
            }),
            Err(other) => Err(other),
        }
    }
}

struct RemoteRef {
    name: String,
    is_tag: bool,
    commit: String,
}

pub struct Remote {
    refs: Vec<RemoteRef>,
    head_branch: Option<String>,
    head: Option<String>,
}

fn is_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Remote {
    /// A branch floats; a tag or a SHA is pinned.
    pub fn floats(&self, git_ref: &str) -> bool {
        git_ref == "HEAD" || self.refs.iter().any(|r| !r.is_tag && r.name == git_ref)
    }

    /// The commit `git_ref` names now, if the remote still has it.
    pub fn commit_of(&self, git_ref: &str) -> Option<String> {
        if git_ref == "HEAD" {
            return self.head.clone();
        }
        let named = |tag: bool| {
            self.refs
                .iter()
                .find(|r| r.is_tag == tag && r.name == git_ref)
        };
        named(false)
            .or_else(|| named(true))
            .map(|r| r.commit.clone())
            .or_else(|| is_sha(git_ref).then(|| git_ref.to_string()))
    }
}

pub async fn ls_remote(git: &Git<'_>, url: &str) -> Result<Remote> {
    let out = git
        .run(
            git.staging,
            &[
                "ls-remote",
                "--symref",
                url,
                "HEAD",
                "refs/heads/*",
                "refs/tags/*",
            ],
            LS_REMOTE_TIMEOUT,
        )
        .await?;
    let mut remote = Remote {
        refs: Vec::new(),
        head_branch: None,
        head: None,
    };
    // A mise shim can print a banner first, so only well-formed lines count.
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("ref: ") {
            let target = rest.split('\t').next().unwrap_or_default();
            remote.head_branch = target.strip_prefix("refs/heads/").map(str::to_string);
            continue;
        }
        let Some((sha, name)) = line.split_once('\t') else {
            continue;
        };
        if !is_sha(sha) {
            continue;
        }
        if name == "HEAD" {
            remote.head = Some(sha.to_string());
        } else if let Some(branch) = name.strip_prefix("refs/heads/") {
            remote.refs.push(RemoteRef {
                name: branch.to_string(),
                is_tag: false,
                commit: sha.to_string(),
            });
        } else if let Some(tag) = name.strip_prefix("refs/tags/") {
            // An annotated tag's peeled line names the commit.
            let (tag, peeled) = match tag.strip_suffix("^{}") {
                Some(tag) => (tag, true),
                None => (tag, false),
            };
            match remote.refs.iter_mut().find(|r| r.is_tag && r.name == tag) {
                Some(existing) if peeled => existing.commit = sha.to_string(),
                Some(_) => {}
                None => remote.refs.push(RemoteRef {
                    name: tag.to_string(),
                    is_tag: true,
                    commit: sha.to_string(),
                }),
            }
        }
    }
    Ok(remote)
}

pub async fn resolve(git: &Git<'_>, url: &str, source: &SkillSourceRef) -> Result<Resolved> {
    let remote = ls_remote(git, url).await?;
    let default_ref = || -> Result<(String, String)> {
        let commit = remote
            .head
            .clone()
            .ok_or_else(|| CoreError::SkillFetchFailed {
                detail: "The repository has no default branch.".into(),
            })?;
        Ok((
            remote.head_branch.clone().unwrap_or_else(|| "HEAD".into()),
            commit,
        ))
    };
    let Some(rest) = &source.ref_and_path else {
        let (git_ref, commit) = default_ref()?;
        return Ok(Resolved {
            git_ref,
            commit,
            path: source.path.clone(),
            name_hint: source.name_hint.clone(),
        });
    };

    // Longest name that is a segment-prefix, so `release/v2/skills/x` finds `release/v2`.
    let best = remote
        .refs
        .iter()
        .filter(|r| {
            *rest == r.name
                || rest
                    .strip_prefix(&r.name)
                    .is_some_and(|t| t.starts_with('/'))
        })
        .max_by_key(|r| (r.name.len(), !r.is_tag));
    if let Some(found) = best {
        let path = rest[found.name.len()..].trim_start_matches('/');
        return Ok(Resolved {
            git_ref: found.name.clone(),
            commit: found.commit.clone(),
            path: (!path.is_empty()).then(|| path.to_string()),
            name_hint: None,
        });
    }
    let (first, tail) = rest.split_once('/').unwrap_or((rest, ""));
    if is_sha(first) {
        return Ok(Resolved {
            git_ref: first.to_string(),
            commit: first.to_string(),
            path: (!tail.is_empty()).then(|| tail.to_string()),
            name_hint: None,
        });
    }
    Err(CoreError::SkillFetchFailed {
        detail: format!("No branch, tag or commit matches {rest}."),
    })
}

/// Recipe steps 2 to 6: a shallow, trees-only fetch, caps enforced before blobs move.
pub async fn fetch_into(
    git: &Git<'_>,
    url: &str,
    resolved: &Resolved,
    staging: &Path,
    filter: bool,
) -> Result<Fetched> {
    let repo_dir = staging.join("repo");
    std::fs::create_dir_all(&repo_dir)?;
    git.run(&repo_dir, &["init", "-q"], LOCAL_TIMEOUT).await?;
    git.run(&repo_dir, &["remote", "add", "origin", url], LOCAL_TIMEOUT)
        .await?;
    let commit = resolved.commit.as_str();
    let mut fetch = vec!["-c", "protocol.version=2", "fetch", "-q", "--depth", "1"];
    if filter {
        fetch.push("--filter=blob:none");
    }
    fetch.extend(["--no-tags", "origin", commit]);
    git.run(&repo_dir, &fetch, FETCH_TIMEOUT).await?;

    let listed = match resolved.path.as_deref() {
        Some(path) => {
            git.run(
                &repo_dir,
                &["ls-tree", "-r", "-z", commit, "--", path],
                LOCAL_TIMEOUT,
            )
            .await?
        }
        None => {
            git.run(&repo_dir, &["ls-tree", "-r", "-z", commit], LOCAL_TIMEOUT)
                .await?
        }
    };
    let regular = parse_tree(&listed);
    let path = match &resolved.path {
        Some(path) => path.clone(),
        None => match resolved.name_hint.as_deref() {
            Some(hint) => pick_hinted(git, &repo_dir, commit, &regular, hint).await?,
            None => discover(&regular)?,
        },
    };

    let prefix = if path.is_empty() {
        String::new()
    } else {
        format!("{path}/")
    };
    let mut entries: Vec<TreeEntry> = regular
        .iter()
        .filter_map(|(full, executable)| {
            full.strip_prefix(&prefix).map(|rel| TreeEntry {
                rel_path: rel.to_string(),
                executable: *executable,
                bytes: 0,
            })
        })
        .collect();
    entries.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    if entries.len() > MAX_FILES {
        return Err(CoreError::SkillTooLarge {
            files: entries.len(),
            bytes: 0,
        });
    }
    if !entries.iter().any(|e| e.rel_path == "SKILL.md") {
        return Err(not_found(&path));
    }

    let target = if path.is_empty() { "." } else { path.as_str() };
    git.run(
        &repo_dir,
        &[
            "-c",
            "core.symlinks=false",
            "checkout",
            "-q",
            commit,
            "--",
            target,
        ],
        CHECKOUT_TIMEOUT,
    )
    .await?;

    let mut total = 0u64;
    for entry in &mut entries {
        let file = repo_dir.join(&path).join(&entry.rel_path);
        entry.bytes = std::fs::metadata(&file)?.len();
        total += entry.bytes;
    }
    if total > MAX_BYTES {
        return Err(CoreError::SkillTooLarge {
            files: entries.len(),
            bytes: total,
        });
    }
    Ok(Fetched {
        repo_dir,
        path,
        entries,
    })
}

/// Regular files only (`100644`/`100755`) as `(path, executable)`; links and submodules drop out.
fn parse_tree(listed: &str) -> Vec<(String, bool)> {
    listed
        .split('\0')
        .filter_map(|record| {
            let (meta, path) = record.split_once('\t')?;
            let mode = meta.split(' ').next()?;
            match mode {
                "100644" => Some((path.to_string(), false)),
                "100755" => Some((path.to_string(), true)),
                _ => None,
            }
        })
        .collect()
}

fn discover(files: &[(String, bool)]) -> Result<String> {
    if files.iter().any(|(p, _)| p == "SKILL.md") {
        return Ok(String::new());
    }
    let mut found: Vec<&str> = files
        .iter()
        .filter_map(|(p, _)| p.strip_suffix("/SKILL.md"))
        .collect();
    found.sort_unstable();
    match found.as_slice() {
        [one] => Ok((*one).to_string()),
        [] => Err(CoreError::SkillNotFoundInSource {
            detail: "No SKILL.md in that repository.".into(),
        }),
        many => Err(CoreError::SkillNotFoundInSource {
            detail: format!(
                "That repository holds {} skills; link to one: {}.",
                many.len(),
                many.iter().take(5).copied().collect::<Vec<_>>().join(", ")
            ),
        }),
    }
}

/// A skills.sh pick: the folder named `hint`, else the one whose frontmatter `name` is it,
/// else a root `SKILL.md`.
async fn pick_hinted(
    git: &Git<'_>,
    repo_dir: &Path,
    commit: &str,
    files: &[(String, bool)],
    hint: &str,
) -> Result<String> {
    let mut dirs: Vec<&str> = files
        .iter()
        .filter_map(|(p, _)| p.strip_suffix("/SKILL.md"))
        .collect();
    dirs.sort_unstable();
    let mut matches: Vec<&str> = dirs
        .iter()
        .copied()
        .filter(|d| d.rsplit('/').next() == Some(hint))
        .collect();
    if matches.is_empty() {
        for dir in dirs.iter().take(MAX_HINT_CANDIDATES) {
            let spec = format!("{commit}:{dir}/SKILL.md");
            let Ok(text) = git
                .run(repo_dir, &["cat-file", "blob", &spec], LOCAL_TIMEOUT)
                .await
            else {
                continue;
            };
            let parsed = crate::docs::frontmatter::parse(&text);
            if parsed.fm.extra.get("name").map(|n| n.trim()) == Some(hint) {
                matches.push(dir);
            }
        }
    }
    match matches.as_slice() {
        [one] => Ok((*one).to_string()),
        // A single-skill repo keeps SKILL.md at its root, outside any `<name>/` folder.
        [] if files.iter().any(|(p, _)| p == "SKILL.md") => Ok(String::new()),
        [] => Err(CoreError::SkillNotFoundInSource {
            detail: format!("No skill named {hint} in that repository."),
        }),
        many => Err(CoreError::SkillNotFoundInSource {
            detail: format!(
                "{} skills in that repository are named {hint}: {}.",
                many.len(),
                many.iter().take(5).copied().collect::<Vec<_>>().join(", ")
            ),
        }),
    }
}

fn not_found(path: &str) -> CoreError {
    CoreError::SkillNotFoundInSource {
        detail: format!(
            "No SKILL.md at {}.",
            if path.is_empty() {
                "the repository root"
            } else {
                path
            }
        ),
    }
}
