//! Moves thoughts docs' `status` at the pipeline milestones the server sees.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::ConfigService;
use crate::docs::lifecycle::{self, Milestone};
use crate::docs::{frontmatter, pr_record, DocType};
use crate::index::roots::{canon_roots, thoughts_roots};
use crate::pr::PrState;
use crate::store::Db;

use super::attribution::doc_id_for_tool_path;

fn milestone_of(state: PrState) -> Option<Milestone> {
    match state {
        PrState::Merged => Some(Milestone::Merged),
        PrState::Closed => Some(Milestone::Closed),
        PrState::Open => None,
    }
}

async fn records_dir(config: &ConfigService, project_path: &str) -> Option<PathBuf> {
    let loaded = config.load().await.ok()?;
    let effective = loaded
        .config
        .thoughts
        .as_ref()?
        .effective_config_for(project_path);
    let root = effective.backend.thoughts_root()?;
    let (repos_dir, _) = effective.backend.tree_dirs()?;
    Some(pr_record::records_dir(
        &root,
        repos_dir,
        effective.mapped_name.as_deref()?,
    ))
}

/// Settles the PR's record and every doc whose newest write was the workstream's; a plan
/// another workstream has written since is that one's to settle.
pub(crate) async fn settle_workstream(
    db: Arc<Db>,
    config: &ConfigService,
    workstream_id: &str,
    project_path: &str,
    number: u32,
    state: PrState,
) {
    if let Some(milestone) = milestone_of(state) {
        settle(
            db,
            config,
            workstream_id,
            project_path,
            number,
            milestone,
            false,
        )
        .await;
    }
}

/// Only the record of a merged PR: describe_pr can write it after the merge, nothing else lands late.
pub(crate) async fn settle_merged_record(
    db: Arc<Db>,
    config: &ConfigService,
    workstream_id: &str,
    project_path: &str,
    number: u32,
) {
    settle(
        db,
        config,
        workstream_id,
        project_path,
        number,
        Milestone::Merged,
        true,
    )
    .await;
}

async fn settle(
    db: Arc<Db>,
    config: &ConfigService,
    workstream_id: &str,
    project_path: &str,
    number: u32,
    milestone: Milestone,
    record_only: bool,
) {
    let record = records_dir(config, project_path)
        .await
        .map(|dir| dir.join(pr_record::file_name(number)));
    let id = workstream_id.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        let written = db.docs_last_written_by(&id).unwrap_or_else(|e| {
            tracing::warn!("workstream {id}: the docs its agents wrote are unreadable: {e}");
            Vec::new()
        });
        let paths: BTreeSet<PathBuf> = record
            .into_iter()
            .chain(written.into_iter().map(PathBuf::from))
            .filter(|path| !record_only || pr_record::record_number(path) == Some(number))
            .collect();
        for path in &paths {
            // Another PR's record this workstream touched is that PR's to settle.
            let admit = |t: DocType| match t {
                DocType::Pr => pr_record::record_number(path) == Some(number),
                _ => !record_only,
            };
            settle_logged(path, milestone, admit, number);
        }
    })
    .await;
}

fn settle_logged(path: &Path, milestone: Milestone, admit: impl Fn(DocType) -> bool, number: u32) {
    match lifecycle::settle_file(path, milestone, admit) {
        Ok(Some(status)) => tracing::info!(
            "PR #{number} {milestone:?}: {} is now {}",
            path.display(),
            status.as_str()
        ),
        Ok(None) => {}
        Err(e) => tracing::warn!("PR #{number}: settling {} failed: {e}", path.display()),
    }
}

/// Settles one PR record found without a workstream.
pub(crate) async fn settle_record(path: PathBuf, number: u32, state: PrState) {
    let Some(milestone) = milestone_of(state) else {
        return;
    };
    let _ = tokio::task::spawn_blocking(move || {
        settle_logged(&path, milestone, |t| t == DocType::Pr, number)
    })
    .await;
}

/// Activates the draft plans a session builds from: its attachments and `.md` paths in its input.
pub(crate) async fn activate_plans(
    config: &ConfigService,
    cwd: &str,
    attached: &[String],
    input: &str,
) {
    let Ok(loaded) = config.load().await else {
        return;
    };
    let roots = thoughts_roots(&loaded.config);
    if roots.is_empty() {
        return;
    }
    let typed = input
        .split_whitespace()
        .map(|t| t.trim_matches(|c| matches!(c, '`' | '"' | '\'' | '(' | ')' | '<' | '>' | ',')))
        .filter(|t| t.ends_with(".md"));
    let candidates: Vec<String> = attached
        .iter()
        .map(String::as_str)
        .chain(typed)
        .map(str::to_string)
        .collect();
    let cwd = PathBuf::from(cwd);
    let _ = tokio::task::spawn_blocking(move || {
        let roots = canon_roots(&roots);
        let ids: BTreeSet<String> = candidates
            .iter()
            .filter_map(|raw| doc_id_for_tool_path(&roots, &cwd, raw))
            .collect();
        for id in ids {
            match lifecycle::settle_file(Path::new(&id), Milestone::ImplementStarted, |t| {
                t == DocType::Plan
            }) {
                Ok(Some(_)) => tracing::info!("implement started: {id} is now active"),
                Ok(None) => {}
                Err(e) => tracing::warn!("implement started: activating {id} failed: {e}"),
            }
        }
    })
    .await;
}

/// A PR record a merge or close could still move.
pub(crate) struct UnsettledRecord {
    pub project_path: String,
    pub path: PathBuf,
    pub number: u32,
}

/// Every project's unsettled PR records, one pass per records dir.
pub(crate) async fn unsettled_records(db: Arc<Db>, config: &ConfigService) -> Vec<UnsettledRecord> {
    let projects = match tokio::task::spawn_blocking(move || db.list_projects()).await {
        Ok(Ok(projects)) => projects,
        Ok(Err(e)) => {
            tracing::warn!("PR record sweep: listing projects failed: {e}");
            return Vec::new();
        }
        Err(_) => return Vec::new(),
    };
    let mut dirs: Vec<(String, PathBuf)> = Vec::new();
    for project in projects {
        if let Some(dir) = records_dir(config, &project.path).await {
            if !dirs.iter().any(|(_, d)| *d == dir) {
                dirs.push((project.path, dir));
            }
        }
    }
    tokio::task::spawn_blocking(move || {
        dirs.into_iter()
            .flat_map(|(project_path, dir)| {
                let entries = std::fs::read_dir(&dir).into_iter().flatten().flatten();
                let unsettled: Vec<UnsettledRecord> = entries
                    .filter_map(|entry| {
                        let path = entry.path();
                        let number = pr_record::record_number(&path)?;
                        let content = std::fs::read_to_string(&path).ok()?;
                        let status = frontmatter::parse(&content).fm.status;
                        lifecycle::record_unsettled(status.as_deref()).then(|| UnsettledRecord {
                            project_path: project_path.clone(),
                            path,
                            number,
                        })
                    })
                    .collect();
                unsettled
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}
