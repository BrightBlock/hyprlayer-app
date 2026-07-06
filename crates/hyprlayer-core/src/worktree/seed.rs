//! Seed a fresh worktree with untracked files from the canonical repo per the
//! repo's `[worktree].copy` list. Best-effort — callers MUST NOT roll a worktree
//! back when seeding fails.

use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SeedReport {
    /// Repo-relative paths actually written into the worktree.
    pub copied: Vec<String>,
    /// Entries (literal or glob) that matched nothing in the repo.
    pub missing: Vec<String>,
    /// Per-entry errors that didn't prevent other entries from copying.
    pub errors: Vec<SeedError>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SeedError {
    pub entry: String,
    pub message: String,
}

/// Cap on entries visited per glob walk; mirrors `gitops::search_files`.
const MAX_VISITED: usize = 50_000;

/// Returns `true` if the pattern string contains any glob metacharacter.
fn is_glob(p: &str) -> bool {
    p.contains(['*', '?', '[', '{'])
}

/// Validate that `rel` is a safe repo-relative path: not absolute, no `..`,
/// no Windows drive prefix. Mirrors `gitops::safe_join`.
fn safe_rel(rel: &str) -> Option<&Path> {
    let p = Path::new(rel);
    let bad = p.is_absolute()
        || p.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        });
    (!bad).then_some(p)
}

/// Create parent dirs and overwrite `dst` with `src`'s contents. `std::fs::copy`
/// dereferences symlinks on Unix (copies the target's content, not the link).
fn copy_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dst).map(|_| ())
}

/// Recursively copy all files under `repo/dir_rel` into `worktree/dir_rel`.
/// Skips `.git` and `.hyprlayer` entries at any depth. Does not follow symlinks
/// for directories (but does copy symlinked file targets via `std::fs::copy`).
fn copy_tree(repo: &Path, worktree: &Path, dir_rel: &Path, report: &mut SeedReport) {
    let src_dir = repo.join(dir_rel);
    let walker = walkdir::WalkDir::new(&src_dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() > 0 && e.file_type().is_dir() {
                if let Some(name) = e.file_name().to_str() {
                    if name == ".git" || name == ".hyprlayer" {
                        return false;
                    }
                }
            }
            true
        });

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                report.errors.push(SeedError {
                    entry: dir_rel.display().to_string(),
                    message: e.to_string(),
                });
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }

        let file_rel = match entry.path().strip_prefix(repo) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        let dst = worktree.join(&file_rel);
        match copy_file(entry.path(), &dst) {
            Ok(_) => report.copied.push(file_rel.display().to_string()),
            Err(e) => report.errors.push(SeedError {
                entry: file_rel.display().to_string(),
                message: e.to_string(),
            }),
        }
    }
}

/// Seed `worktree` from `repo`, copying every file matched by `patterns`.
/// Literals stat+copy directly; globs drive one bounded `walkdir` (capped at
/// `MAX_VISITED`, skipping `.git/`/`.hyprlayer/`). Best-effort: never panics or
/// errors — failures land in `SeedReport::errors`, non-matches in `missing`.
pub fn seed_worktree(repo: &Path, worktree: &Path, patterns: &[String]) -> SeedReport {
    run(repo, Some(worktree), patterns)
}

/// Dry-run [`seed_worktree`]: resolve `patterns` and report what WOULD be copied
/// / matched nothing (`missing`) / errored, writing nothing. Drives the "copy
/// files not found" warning so a bad copy config surfaces before any worktree.
pub fn check_worktree_patterns(repo: &Path, patterns: &[String]) -> SeedReport {
    run(repo, None, patterns)
}

/// Shared engine. `dst = Some(worktree)` copies; `dst = None` is a dry run that
/// only resolves matches (used by [`check_worktree_patterns`]).
fn run(repo: &Path, dst: Option<&Path>, patterns: &[String]) -> SeedReport {
    let mut report = SeedReport::default();
    if patterns.is_empty() {
        return report;
    }

    let (literals, glob_pats): (Vec<&str>, Vec<&str>) = patterns
        .iter()
        .map(String::as_str)
        .partition(|p| !is_glob(p));

    // ── Literal paths (no walk) ───────────────────────────────────────────────
    for lit in &literals {
        match safe_rel(lit) {
            None => report.errors.push(SeedError {
                entry: lit.to_string(),
                message: "unsafe or absolute path".into(),
            }),
            Some(rel) => {
                let src = repo.join(rel);
                match src.symlink_metadata() {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        report.missing.push(lit.to_string());
                    }
                    Err(e) => report.errors.push(SeedError {
                        entry: lit.to_string(),
                        message: e.to_string(),
                    }),
                    Ok(meta) if meta.is_dir() => match dst {
                        Some(worktree) => copy_tree(repo, worktree, rel, &mut report),
                        // Dry run: an existing directory resolves; no need to walk it.
                        None => report.copied.push(lit.to_string()),
                    },
                    Ok(_) => match dst {
                        // File or symlink — copy target content.
                        Some(worktree) => {
                            let out = worktree.join(rel);
                            match copy_file(&src, &out) {
                                Ok(_) => report.copied.push(lit.to_string()),
                                Err(e) => report.errors.push(SeedError {
                                    entry: lit.to_string(),
                                    message: e.to_string(),
                                }),
                            }
                        }
                        None => report.copied.push(lit.to_string()),
                    },
                }
            }
        }
    }

    if glob_pats.is_empty() {
        return report;
    }

    // ── Glob patterns (one bounded walk) ─────────────────────────────────────
    // Compile each pattern to a GlobMatcher; compile failures go to errors.
    struct GlobEntry {
        pattern: String,
        matcher: globset::GlobMatcher,
        hit: bool,
    }

    let mut matchers: Vec<GlobEntry> = Vec::new();
    for pat in &glob_pats {
        match globset::Glob::new(pat) {
            Ok(g) => matchers.push(GlobEntry {
                pattern: pat.to_string(),
                matcher: g.compile_matcher(),
                hit: false,
            }),
            Err(e) => report.errors.push(SeedError {
                entry: pat.to_string(),
                message: e.to_string(),
            }),
        }
    }

    let walker = walkdir::WalkDir::new(repo)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            // Skip .git and .hyprlayer at any depth > 0.
            if e.depth() > 0 && e.file_type().is_dir() {
                if let Some(name) = e.file_name().to_str() {
                    if name == ".git" || name == ".hyprlayer" {
                        return false;
                    }
                }
            }
            true
        });

    for (idx, entry) in walker.enumerate() {
        if idx >= MAX_VISITED {
            tracing::warn!("worktree seed: glob walk truncated at {MAX_VISITED} entries");
            break;
        }
        // Dry run only needs to know each pattern matches *something*; once every
        // matcher has hit, the rest of the walk can't change the report.
        if dst.is_none() && matchers.iter().all(|ge| ge.hit) {
            break;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }

        let file_rel: PathBuf = match entry.path().strip_prefix(repo) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };

        let mut already_copied = false;
        for ge in matchers.iter_mut() {
            if ge.matcher.is_match(&file_rel) {
                ge.hit = true;
                if let Some(worktree) = dst {
                    if !already_copied {
                        let rel_str = file_rel.display().to_string();
                        let out = worktree.join(&file_rel);
                        match copy_file(entry.path(), &out) {
                            Ok(_) => {
                                report.copied.push(rel_str);
                                already_copied = true;
                            }
                            Err(e) => {
                                report.errors.push(SeedError {
                                    entry: rel_str,
                                    message: e.to_string(),
                                });
                                already_copied = true; // don't attempt again for this file
                            }
                        }
                    }
                }
            }
        }
    }

    for ge in &matchers {
        if !ge.hit {
            report.missing.push(ge.pattern.clone());
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Create a temp dir pair (repo, worktree).
    fn dirs() -> (TempDir, TempDir) {
        (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
    }

    fn write(root: &Path, rel: &str, content: &[u8]) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
    }

    // ── Core cases ────────────────────────────────────────────────────────────

    #[test]
    fn empty_patterns_returns_empty_report() {
        let (repo, wt) = dirs();
        write(repo.path(), ".env", b"SECRET=1");
        let r = seed_worktree(repo.path(), wt.path(), &[]);
        assert!(r.copied.is_empty());
        assert!(r.missing.is_empty());
        assert!(r.errors.is_empty());
    }

    #[test]
    fn literal_file_copied_to_same_relative_path() {
        let (repo, wt) = dirs();
        write(repo.path(), ".env", b"SECRET=1");
        let r = seed_worktree(repo.path(), wt.path(), &[".env".into()]);
        assert_eq!(r.copied, vec![".env"]);
        assert!(r.missing.is_empty());
        assert!(r.errors.is_empty());
        assert_eq!(fs::read(wt.path().join(".env")).unwrap(), b"SECRET=1");
    }

    #[test]
    fn literal_nested_file_creates_parent_dirs() {
        let (repo, wt) = dirs();
        write(repo.path(), "config/local.json", b"{}");
        let r = seed_worktree(repo.path(), wt.path(), &["config/local.json".into()]);
        assert_eq!(r.copied, vec!["config/local.json"]);
        assert!(wt.path().join("config/local.json").exists());
    }

    #[test]
    fn literal_directory_copies_recursively() {
        let (repo, wt) = dirs();
        write(repo.path(), "certs/ca.crt", b"CERT");
        write(repo.path(), "certs/server.key", b"KEY");
        let r = seed_worktree(repo.path(), wt.path(), &["certs".into()]);
        assert_eq!(r.errors, vec![], "no errors expected");
        let mut copied = r.copied.clone();
        copied.sort();
        assert_eq!(copied, vec!["certs/ca.crt", "certs/server.key"]);
        assert_eq!(fs::read(wt.path().join("certs/ca.crt")).unwrap(), b"CERT");
    }

    #[test]
    fn missing_literal_goes_to_missing_not_errors() {
        let (repo, wt) = dirs();
        let r = seed_worktree(repo.path(), wt.path(), &[".env".into()]);
        assert!(r.copied.is_empty());
        assert_eq!(r.missing, vec![".env"]);
        assert!(r.errors.is_empty());
    }

    #[test]
    fn existing_destination_is_overwritten() {
        let (repo, wt) = dirs();
        write(repo.path(), ".env", b"NEW");
        write(wt.path(), ".env", b"OLD");
        let r = seed_worktree(repo.path(), wt.path(), &[".env".into()]);
        assert_eq!(r.copied, vec![".env"]);
        assert_eq!(fs::read(wt.path().join(".env")).unwrap(), b"NEW");
    }

    // ── Path-safety ───────────────────────────────────────────────────────────

    #[test]
    fn path_escape_parent_dir_goes_to_errors() {
        let (repo, wt) = dirs();
        let r = seed_worktree(repo.path(), wt.path(), &["../escape".into()]);
        assert!(r.errors.iter().any(|e| e.entry == "../escape"), "{r:?}");
        assert!(r.copied.is_empty());
    }

    #[test]
    fn absolute_path_goes_to_errors() {
        let (repo, wt) = dirs();
        let r = seed_worktree(repo.path(), wt.path(), &["/etc/passwd".into()]);
        assert!(r.errors.iter().any(|e| e.entry == "/etc/passwd"), "{r:?}");
        assert!(r.copied.is_empty());
    }

    // ── .git / .hyprlayer exclusion ───────────────────────────────────────────

    #[test]
    fn git_dir_not_copied_by_glob() {
        let (repo, wt) = dirs();
        write(repo.path(), ".git/config", b"[core]");
        write(repo.path(), "real.txt", b"real");
        // A glob that would match .git/config if not excluded
        let r = seed_worktree(repo.path(), wt.path(), &["**/*".into()]);
        assert!(
            r.copied.iter().all(|p| !p.contains(".git")),
            ".git must not be copied: {r:?}"
        );
    }

    #[test]
    fn hyprlayer_dir_not_copied_by_glob() {
        let (repo, wt) = dirs();
        write(repo.path(), ".hyprlayer/settings.toml", b"[worktree]");
        write(repo.path(), "real.txt", b"real");
        let r = seed_worktree(repo.path(), wt.path(), &["**/*".into()]);
        assert!(
            r.copied.iter().all(|p| !p.contains(".hyprlayer")),
            ".hyprlayer must not be copied: {r:?}"
        );
    }

    // ── Glob matching ─────────────────────────────────────────────────────────

    #[test]
    fn glob_copies_all_matches() {
        let (repo, wt) = dirs();
        write(repo.path(), "a.env", b"A");
        write(repo.path(), "b.env", b"B");
        write(repo.path(), "c.txt", b"C"); // should NOT match
        let r = seed_worktree(repo.path(), wt.path(), &["*.env".into()]);
        let mut copied = r.copied.clone();
        copied.sort();
        assert_eq!(copied, vec!["a.env", "b.env"]);
        assert!(!wt.path().join("c.txt").exists());
    }

    #[test]
    fn glob_with_no_matches_goes_to_missing() {
        let (repo, wt) = dirs();
        write(repo.path(), "readme.txt", b"hi");
        let r = seed_worktree(repo.path(), wt.path(), &["*.env".into()]);
        assert!(r.copied.is_empty());
        assert_eq!(r.missing, vec!["*.env"]);
        assert!(r.errors.is_empty());
    }

    #[test]
    fn glob_with_path_separator() {
        let (repo, wt) = dirs();
        write(repo.path(), "config/a.local.json", b"{}");
        write(repo.path(), "config/b.local.json", b"{}");
        write(repo.path(), "config/prod.json", b"{}"); // should NOT match
        let r = seed_worktree(repo.path(), wt.path(), &["config/*.local.json".into()]);
        let mut copied = r.copied.clone();
        copied.sort();
        assert_eq!(copied, vec!["config/a.local.json", "config/b.local.json"]);
    }

    // ── Dry-run check (check_worktree_patterns) ───────────────────────────────

    #[test]
    fn check_present_nested_literal_is_clean_and_writes_nothing() {
        let (repo, wt) = dirs();
        write(repo.path(), "tools/operating-review/.env", b"SECRET=1");
        let r = check_worktree_patterns(repo.path(), &["tools/operating-review/.env".into()]);
        assert!(r.missing.is_empty(), "should resolve: {r:?}");
        assert!(r.errors.is_empty());
        // A check must never touch the filesystem.
        assert!(!wt.path().join("tools/operating-review/.env").exists());
    }

    #[test]
    fn check_absent_literal_is_missing() {
        let (repo, _wt) = dirs();
        let r = check_worktree_patterns(repo.path(), &["tools/operating-review/.env".into()]);
        assert_eq!(r.missing, vec!["tools/operating-review/.env"]);
        assert!(r.copied.is_empty());
        assert!(r.errors.is_empty());
    }

    #[test]
    fn check_existing_directory_resolves() {
        let (repo, _wt) = dirs();
        write(repo.path(), "certs/ca.crt", b"CERT");
        let r = check_worktree_patterns(repo.path(), &["certs".into()]);
        assert!(r.missing.is_empty(), "existing dir resolves: {r:?}");
    }

    #[test]
    fn check_glob_hit_and_miss() {
        let (repo, _wt) = dirs();
        write(repo.path(), "pkg/web/.env", b"A");
        assert!(
            check_worktree_patterns(repo.path(), &["**/.env".into()])
                .missing
                .is_empty()
        );
        assert_eq!(
            check_worktree_patterns(repo.path(), &["**/*.secret".into()]).missing,
            vec!["**/*.secret"]
        );
    }

    #[test]
    fn check_unsafe_path_is_error_not_missing() {
        let (repo, _wt) = dirs();
        let r = check_worktree_patterns(repo.path(), &["../escape".into()]);
        assert!(r.errors.iter().any(|e| e.entry == "../escape"), "{r:?}");
        assert!(r.missing.is_empty());
    }
}
