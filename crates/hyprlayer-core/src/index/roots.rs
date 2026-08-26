use std::path::PathBuf;

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
            ai: None,
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
