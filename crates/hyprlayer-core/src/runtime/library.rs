use std::path::{Path, PathBuf};

use hyprlayer_models::runtime::ThoughtsIndexUpdateKind;
use hyprlayer_models::RuntimeEvent;

use crate::config::{BackendConfig, EffectiveConfig};
use crate::docs::frontmatter::{patch_frontmatter, FmPatch};
use crate::docs::parts::{self, compose};
use crate::docs::templates::{new_doc, DocTemplate};
use crate::error::{CoreError, Result};
use crate::gitops::{self, ThoughtsRepoStatus};
use crate::index::related;
use crate::index::{
    thoughts_roots, unindexed_meta, DocMeta, QueryResult, ThoughtQuery, ThoughtsRoot,
};
use serde::{Deserialize, Serialize};

use super::HyprlayerRuntime;

pub use crate::docs::parts::FrontmatterEdit;
pub use crate::index::related::{RelatedLink, RelatedTarget};

fn ensure_running(runtime: &HyprlayerRuntime) -> Result<()> {
    if runtime.coordinator.is_stopped() {
        return Err(CoreError::RuntimeStopped);
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_err) => Err(CoreError::Io(std::io::Error::other(join_err.to_string()))),
    }
}

fn io_context(label: &str, err: std::io::Error) -> CoreError {
    CoreError::Io(std::io::Error::other(format!("{label}: {err}")))
}

fn file_mtime_ms(path: &Path) -> f64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

// The tolerance absorbs the round trip through an f64 millisecond wire value;
// anything larger is a real external edit.
pub(super) fn ensure_not_stale(abs: &Path, base_mtime_ms: f64) -> Result<()> {
    if (file_mtime_ms(abs) - base_mtime_ms).abs() > 0.5 {
        return Err(CoreError::DocConflict);
    }
    Ok(())
}

fn document_from(meta: DocMeta, content: String, mtime_ms: f64) -> Document {
    let split = parts::split(&content);
    let body = split.body(&content).to_string();
    let frontmatter = split.block.map(str::to_string);
    let fm_error = split.fm_error;
    let fm_keys = split.keys;
    Document {
        meta,
        content,
        mtime_ms,
        body,
        frontmatter,
        fm_error,
        fm_keys,
    }
}

// `body`/`frontmatter`/`fm_error`/`fm_keys` are deliberately not
// `#[serde(default)]`: an app talking to an older server must fail to decode
// rather than get an empty `body` and later save it over the file.
// `MIN_SERVER_VERSION` keeps released servers (<= 0.8.4) out.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub meta: DocMeta,
    pub content: String,
    pub mtime_ms: f64,
    pub body: String,
    pub frontmatter: Option<String>,
    pub fm_error: Option<String>,
    pub fm_keys: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateDocumentRequest {
    pub project_repo_path: Option<String>,
    pub scope: String,
    pub doc_type: String,
    pub title: String,
    pub date: String,
    pub ticket: Option<String>,
    pub tags: Vec<String>,
}

pub struct LibraryService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn library(&self) -> LibraryService<'_> {
        LibraryService { runtime: self }
    }
}

impl LibraryService<'_> {
    async fn effective_for(&self, project_path: &str) -> Result<EffectiveConfig> {
        let loaded = self.runtime.config.load().await?;
        let thoughts = loaded
            .config
            .thoughts
            .as_ref()
            .ok_or(CoreError::ThoughtsNotConfigured)?;
        Ok(thoughts.effective_config_for(project_path))
    }

    async fn root_for_doc(&self, abs: &Path) -> Result<ThoughtsRoot> {
        let loaded = self.runtime.config.load().await?;
        thoughts_roots(&loaded.config)
            .into_iter()
            .find(|r| abs.starts_with(&r.root))
            .ok_or_else(|| CoreError::DocOutsideRoots {
                path: abs.display().to_string(),
            })
    }

    async fn write_and_reindex(
        &self,
        abs: PathBuf,
        root: ThoughtsRoot,
        content: String,
    ) -> Result<DocMeta> {
        let index = self.runtime.index.clone();
        let id = blocking(move || {
            let tmp = abs.with_extension("md.tmp");
            std::fs::write(&tmp, &content).map_err(|e| io_context("write doc", e))?;
            std::fs::rename(&tmp, &abs).map_err(|e| io_context("write doc", e))?;
            let written = index.upsert_path(&root, &abs);
            Ok(written.or_else(|| index.resolve_id(&abs.to_string_lossy())))
        })
        .await?;

        let id = id.ok_or(CoreError::DocNotIndexable)?;
        let meta = self
            .runtime
            .index
            .get(&id)
            .ok_or_else(|| CoreError::DocNotIndexed { id: id.clone() })?;
        let _ = self
            .runtime
            .events
            .send(RuntimeEvent::ThoughtsIndexUpdated {
                update: ThoughtsIndexUpdateKind::Updated,
                ids: vec![id],
            });
        Ok(meta)
    }

    /// A document id is a path, but not necessarily the spelling the index
    /// holds it under: a caller may hand over an uncanonicalized one.
    pub fn resolve_path(&self, id: &str) -> Result<PathBuf> {
        let resolved = self
            .runtime
            .index
            .resolve_id(id)
            .ok_or_else(|| CoreError::DocNotIndexed { id: id.to_string() })?;
        self.runtime
            .index
            .doc_path(&resolved)
            .ok_or(CoreError::DocNotIndexed { id: resolved })
    }

    pub async fn query(&self, query: ThoughtQuery) -> Result<QueryResult> {
        ensure_running(self.runtime)?;
        let index = self.runtime.index.clone();
        blocking(move || Ok(index.query(&query))).await
    }

    pub async fn get(&self, id: &str) -> Result<Document> {
        ensure_running(self.runtime)?;
        if let Some(resolved) = self.runtime.index.resolve_id(id) {
            if let Some(meta) = self.runtime.index.get(&resolved) {
                let path = PathBuf::from(&resolved);
                let content = tokio::fs::read_to_string(&path)
                    .await
                    .map_err(|e| io_context("read doc", e))?;
                let mtime_ms = file_mtime_ms(&path);
                return Ok(document_from(meta, content, mtime_ms));
            }
        }

        let abs = PathBuf::from(id);
        if !abs
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            return Err(CoreError::DocNotIndexed { id: id.to_string() });
        }
        let content = tokio::fs::read_to_string(&abs)
            .await
            .map_err(|_| CoreError::DocNotIndexed { id: id.to_string() })?;
        let meta = unindexed_meta(&abs, &content);
        let mtime_ms = file_mtime_ms(&abs);
        Ok(document_from(meta, content, mtime_ms))
    }

    pub async fn save(&self, id: &str, content: String, base_mtime_ms: f64) -> Result<DocMeta> {
        ensure_running(self.runtime)?;
        let abs = self.resolve_path(id)?;
        ensure_not_stale(&abs, base_mtime_ms)?;
        let root = self.root_for_doc(&abs).await?;
        self.write_and_reindex(abs, root, content).await
    }

    pub async fn patch_frontmatter(
        &self,
        id: &str,
        patch: FmPatch,
        base_mtime_ms: f64,
    ) -> Result<DocMeta> {
        ensure_running(self.runtime)?;
        let abs = self.resolve_path(id)?;
        ensure_not_stale(&abs, base_mtime_ms)?;
        let original = tokio::fs::read_to_string(&abs)
            .await
            .map_err(|e| io_context("read doc", e))?;

        let patched = patch_frontmatter(&original, &patch)?;

        let root = self.root_for_doc(&abs).await?;
        self.write_and_reindex(abs, root, patched).await
    }

    pub async fn save_parts(
        &self,
        id: &str,
        body: String,
        edit: FrontmatterEdit,
        base_mtime_ms: f64,
    ) -> Result<DocMeta> {
        ensure_running(self.runtime)?;
        let abs = self.resolve_path(id)?;
        ensure_not_stale(&abs, base_mtime_ms)?;
        let original = tokio::fs::read_to_string(&abs)
            .await
            .map_err(|e| io_context("read doc", e))?;
        let composed = compose(&original, &body, &edit)?;
        if composed == original {
            // No write: bytes, mtime and the watcher stay put.
            let key = abs.to_string_lossy().into_owned();
            return self
                .runtime
                .index
                .get(&key)
                .ok_or(CoreError::DocNotIndexed { id: key });
        }
        let root = self.root_for_doc(&abs).await?;
        self.write_and_reindex(abs, root, composed).await
    }

    pub async fn resolve_related(&self, id: &str, values: Vec<String>) -> Result<Vec<RelatedLink>> {
        ensure_running(self.runtime)?;
        let abs = self.resolve_path(id)?;
        let key = abs.to_string_lossy().into_owned();
        let source = self
            .runtime
            .index
            .get(&key)
            .ok_or_else(|| CoreError::DocNotIndexed { id: key.clone() })?;
        let root = self.root_for_doc(&abs).await?;
        let index = self.runtime.index.clone();
        blocking(move || {
            let docs = index.query(&ThoughtQuery::default()).items;
            Ok(related::resolve(&source, &root, &values, &docs, |p| {
                index.resolve_id(&p.to_string_lossy())
            }))
        })
        .await
    }

    pub async fn create(&self, req: CreateDocumentRequest) -> Result<DocMeta> {
        ensure_running(self.runtime)?;
        let loaded = self.runtime.config.load().await?;
        let thoughts = loaded
            .config
            .thoughts
            .as_ref()
            .ok_or(CoreError::ThoughtsNotConfigured)?;

        let (effective, mapped_name) = match (&req.project_repo_path, req.scope.as_str()) {
            (_, "global") | (None, _) => (thoughts.effective_config_for(""), None),
            (Some(repo), _) => {
                let eff = thoughts.effective_config_for(repo);
                let mapped = eff.mapped_name.clone();
                if mapped.is_none() {
                    return Err(CoreError::ProjectNotFound { path: repo.clone() });
                }
                (eff, mapped)
            }
        };

        let root =
            effective
                .backend
                .thoughts_root()
                .ok_or_else(|| CoreError::BackendNotFilesystem {
                    backend: effective.backend.kind().as_str().to_string(),
                })?;

        let (repos_dir, global_dir) = match &effective.backend {
            BackendConfig::Git(g) => (g.repos_dir.clone(), g.global_dir.clone()),
            BackendConfig::Obsidian(o) => (o.repos_dir.clone(), o.global_dir.clone()),
            _ => unreachable!("thoughts_root() returned Some for a flat-list backend"),
        };
        let area_dir = match req.scope.as_str() {
            "shared" | "global" => "shared".to_string(),
            _ => effective.user.clone(),
        };
        let type_dir = match req.doc_type.as_str() {
            "plan" => "plans",
            "research" => "research",
            "handoff" => "handoffs",
            "pr" => "prs",
            _ => "notes",
        };
        let dir = if req.scope == "global" {
            root.join(&global_dir).join(&area_dir).join(type_dir)
        } else {
            let mapped = mapped_name.as_deref().unwrap_or_default();
            root.join(&repos_dir)
                .join(mapped)
                .join(&area_dir)
                .join(type_dir)
        };

        let template = DocTemplate {
            title: req.title,
            doc_type: req.doc_type,
            date: req.date,
            ticket: req.ticket,
            project: mapped_name,
            scope: req.scope,
            author: effective.user,
            tags: req.tags,
        };
        let doc = new_doc(&template);

        let abs = dir.join(&doc.file_name);
        if abs.exists() {
            return Err(CoreError::DocExists {
                path: abs.display().to_string(),
            });
        }
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| io_context("create doc dir", e))?;

        let root_info = self.root_for_doc(&abs).await?;
        self.write_and_reindex(abs, root_info, doc.content).await
    }

    pub async fn thoughts_git_status(&self, project_path: &str) -> Result<ThoughtsRepoStatus> {
        ensure_running(self.runtime)?;
        let effective = self.effective_for(project_path).await?;

        match &effective.backend {
            BackendConfig::Git(_) => {}
            other => {
                return Err(CoreError::BackendNotGit {
                    backend: other.kind().as_str().to_string(),
                })
            }
        }
        let root = effective
            .backend
            .thoughts_root()
            .ok_or(CoreError::ThoughtsRepoMissing)?;

        blocking(move || gitops::thoughts_repo_status(&root)).await
    }
}
