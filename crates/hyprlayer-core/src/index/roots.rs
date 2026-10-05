use std::path::{Path, PathBuf};

use crate::config::{BackendConfig, BackendKind, HyprlayerConfig};

#[derive(Debug, Clone, PartialEq)]
pub struct ThoughtsRoot {
    pub root: PathBuf,
    pub repos_dir: String,
    pub global_dir: String,
    pub backend: BackendKind,
    pub profile: Option<String>,
}

pub fn thoughts_roots(config: &HyprlayerConfig) -> Vec<ThoughtsRoot> {
    let Some(thoughts) = config.thoughts.as_ref() else {
        return Vec::new();
    };

    let mut roots: Vec<ThoughtsRoot> = Vec::new();
    let push = |root: Option<ThoughtsRoot>, roots: &mut Vec<ThoughtsRoot>| {
        if let Some(candidate) = root {
            if !roots.iter().any(|r| r.root == candidate.root) {
                roots.push(candidate);
            }
        }
    };

    push(root_for(&thoughts.backend, None), &mut roots);

    let mut names: Vec<&String> = thoughts.profiles.keys().collect();
    names.sort();
    for name in names {
        let profile = &thoughts.profiles[name];
        push(root_for(&profile.backend, Some(name.clone())), &mut roots);
    }

    roots
}

fn root_for(backend: &BackendConfig, profile: Option<String>) -> Option<ThoughtsRoot> {
    if !backend.kind().uses_filesystem() {
        return None;
    }
    let root = backend.thoughts_root()?;
    let (repos_dir, global_dir) = match backend {
        BackendConfig::Git(g) => (g.repos_dir.as_str(), g.global_dir.as_str()),
        BackendConfig::Obsidian(o) => (o.repos_dir.as_str(), o.global_dir.as_str()),
        BackendConfig::Notion(_) | BackendConfig::Anytype(_) => return None,
    };
    Some(ThoughtsRoot {
        root,
        repos_dir: non_empty_or(repos_dir, "repos"),
        global_dir: non_empty_or(global_dir, "global"),
        backend: backend.kind(),
        profile,
    })
}

fn non_empty_or(value: &str, default: &str) -> String {
    if value.is_empty() {
        default.to_string()
    } else {
        value.to_string()
    }
}

pub(crate) struct RootCanon {
    pub(crate) root: ThoughtsRoot,
    pub(crate) canon: Option<PathBuf>,
}

pub(crate) fn canon_roots(roots: &[ThoughtsRoot]) -> Vec<RootCanon> {
    roots
        .iter()
        .map(|root| RootCanon {
            canon: std::fs::canonicalize(&root.root)
                .ok()
                .filter(|c| c != &root.root),
            root: root.clone(),
        })
        .collect()
}

pub(crate) fn rebase_onto_roots<'a>(
    roots: &'a [RootCanon],
    path: &Path,
) -> Option<(&'a ThoughtsRoot, PathBuf)> {
    let mut best: Option<(&ThoughtsRoot, PathBuf, usize)> = None;
    let mut consider = |root: &'a ThoughtsRoot, local: PathBuf, depth: usize| {
        if best.as_ref().is_none_or(|(_, _, d)| depth > *d) {
            best = Some((root, local, depth));
        }
    };
    for w in roots {
        if path.starts_with(&w.root.root) {
            consider(
                &w.root,
                path.to_path_buf(),
                w.root.root.components().count(),
            );
        } else if let Some(canon) = &w.canon {
            if let Ok(rel) = path.strip_prefix(canon) {
                consider(&w.root, w.root.root.join(rel), canon.components().count());
            }
        }
    }
    best.map(|(root, local, _)| (root, local))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GitConfig, ObsidianConfig, ProfileConfig, ThoughtsConfig};

    fn git_backend(repo: &str) -> BackendConfig {
        BackendConfig::Git(GitConfig {
            thoughts_repo: repo.to_string(),
            repos_dir: "repos".into(),
            global_dir: "global".into(),
        })
    }

    fn config_with(thoughts: ThoughtsConfig) -> HyprlayerConfig {
        HyprlayerConfig {
            version: Some(4),
            thoughts: Some(thoughts),
        }
    }

    #[test]
    fn empty_when_unconfigured() {
        assert!(thoughts_roots(&HyprlayerConfig::default()).is_empty());

        let cfg = config_with(ThoughtsConfig {
            user: "jt".into(),
            backend: BackendConfig::Git(GitConfig::default()),
            ..Default::default()
        });
        assert!(thoughts_roots(&cfg).is_empty());
    }

    #[test]
    fn default_backend_then_sorted_profiles_deduped() {
        let mut thoughts = ThoughtsConfig {
            user: "jt".into(),
            backend: git_backend("/tmp/hyprlayer-test-thoughts"),
            ..Default::default()
        };
        thoughts.profiles.insert(
            "dup".into(),
            ProfileConfig {
                backend: git_backend("/tmp/hyprlayer-test-thoughts"),
            },
        );
        thoughts.profiles.insert(
            "cloud".into(),
            ProfileConfig {
                backend: BackendConfig::Notion(Default::default()),
            },
        );
        thoughts.profiles.insert(
            "vault".into(),
            ProfileConfig {
                backend: BackendConfig::Obsidian(ObsidianConfig {
                    vault_path: "/tmp/hyprlayer-test-vault".into(),
                    vault_subpath: Some("hyprlayer".into()),
                    repos_dir: String::new(),
                    global_dir: String::new(),
                }),
            },
        );

        let roots = thoughts_roots(&config_with(thoughts));
        assert_eq!(roots.len(), 2);

        assert_eq!(roots[0].root, PathBuf::from("/tmp/hyprlayer-test-thoughts"));
        assert_eq!(roots[0].backend, BackendKind::Git);
        assert_eq!(roots[0].profile, None);
        assert_eq!(roots[0].repos_dir, "repos");
        assert_eq!(roots[0].global_dir, "global");

        assert_eq!(
            roots[1].root,
            PathBuf::from("/tmp/hyprlayer-test-vault/hyprlayer")
        );
        assert_eq!(roots[1].backend, BackendKind::Obsidian);
        assert_eq!(roots[1].profile.as_deref(), Some("vault"));
        assert_eq!(roots[1].repos_dir, "repos");
        assert_eq!(roots[1].global_dir, "global");
    }

    fn root_at(path: &str) -> ThoughtsRoot {
        ThoughtsRoot {
            root: PathBuf::from(path),
            repos_dir: "repos".into(),
            global_dir: "global".into(),
            backend: BackendKind::Git,
            profile: None,
        }
    }

    #[test]
    fn rebase_onto_roots_prefers_deepest_root_and_rebases_canonical_paths() {
        let roots = vec![
            RootCanon {
                root: root_at("/a"),
                canon: None,
            },
            RootCanon {
                root: root_at("/a/nested"),
                canon: None,
            },
            RootCanon {
                root: root_at("/sym/root"),
                canon: Some(PathBuf::from("/real/root")),
            },
        ];

        let (root, local) = rebase_onto_roots(&roots, Path::new("/a/nested/x.md")).unwrap();
        assert_eq!(root.root, PathBuf::from("/a/nested"));
        assert_eq!(local, PathBuf::from("/a/nested/x.md"));

        let (root, local) = rebase_onto_roots(&roots, Path::new("/real/root/global/x.md")).unwrap();
        assert_eq!(root.root, PathBuf::from("/sym/root"));
        assert_eq!(local, PathBuf::from("/sym/root/global/x.md"));

        assert!(rebase_onto_roots(&roots, Path::new("/elsewhere/x.md")).is_none());
    }

    #[test]
    fn tilde_paths_expand() {
        let cfg = config_with(ThoughtsConfig {
            user: "jt".into(),
            backend: git_backend("~/thoughts"),
            ..Default::default()
        });
        let roots = thoughts_roots(&cfg);
        assert_eq!(roots.len(), 1);
        assert!(roots[0].root.is_absolute());
        assert!(!roots[0].root.to_string_lossy().contains('~'));
    }
}
