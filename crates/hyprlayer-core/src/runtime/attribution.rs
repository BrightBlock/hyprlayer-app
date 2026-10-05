use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::broadcast;

use hyprlayer_models::runtime::DocStateChange;
use hyprlayer_models::RuntimeEvent;

use crate::config::ConfigService;
use crate::error::{CoreError, Result};
use crate::index::roots::{canon_roots, rebase_onto_roots, thoughts_roots, RootCanon};
use crate::index::ThoughtsIndex;
use crate::store::{now_ms, write_tool_paths, Db, DocAgentRow};

const BACKFILL_WINDOW: i64 = 2000;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_err) => Err(CoreError::Io(std::io::Error::other(join_err.to_string()))),
    }
}

/// Paths a normalized write tool touched: `input.file_path` plus every `edits[].file_path`.
pub(crate) fn write_paths(name: &str, input: &str) -> Vec<String> {
    if !matches!(name, "Write" | "Edit" | "MultiEdit") {
        return Vec::new();
    }
    let Ok(input) = serde_json::from_str::<serde_json::Value>(input) else {
        return Vec::new();
    };
    let edits = input
        .get("edits")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    write_tool_paths(
        input.get("file_path").and_then(serde_json::Value::as_str),
        edits,
    )
}

/// The index id a tool path lands on, whether or not the watcher has indexed it yet.
pub(crate) fn doc_id_for_tool_path(roots: &[RootCanon], cwd: &Path, raw: &str) -> Option<String> {
    let joined = cwd.join(raw);
    let path = std::fs::canonicalize(&joined).unwrap_or(joined);
    let (root, local) = rebase_onto_roots(roots, &path)?;
    ThoughtsIndex::classify(root, &local).map(|_| local.to_string_lossy().into_owned())
}

fn doc_ids(roots: &[RootCanon], cwd: &str, paths: &[String]) -> BTreeSet<String> {
    paths
        .iter()
        .filter_map(|raw| doc_id_for_tool_path(roots, Path::new(cwd), raw))
        .collect()
}

/// Records which harness session wrote each thoughts doc.
pub(crate) struct DocAttribution {
    db: Arc<Db>,
    config: Arc<ConfigService>,
    events: broadcast::Sender<RuntimeEvent>,
}

impl DocAttribution {
    pub(crate) fn new(
        db: Arc<Db>,
        config: Arc<ConfigService>,
        events: broadcast::Sender<RuntimeEvent>,
    ) -> Self {
        Self { db, config, events }
    }

    pub(crate) async fn record(self: Arc<Self>, session_id: String, paths: Vec<String>) {
        if let Err(e) = self.try_record(&session_id, paths).await {
            tracing::warn!("session {session_id}: doc attribution not recorded: {e}");
        }
    }

    async fn try_record(&self, session_id: &str, paths: Vec<String>) -> Result<()> {
        let roots = thoughts_roots(&self.config.load().await?.config);
        if roots.is_empty() {
            return Ok(());
        }
        let db = self.db.clone();
        let session_id = session_id.to_string();
        let changed = blocking(move || {
            let Some(session) = db.get_session(&session_id)? else {
                return Ok(Vec::new());
            };
            let roots = canon_roots(&roots);
            let at_ms = now_ms();
            let mut changed = Vec::new();
            for doc_id in doc_ids(&roots, &session.cwd, &paths) {
                let row = DocAgentRow {
                    doc_id,
                    session_id: session.id.clone(),
                    harness: session.harness,
                    workstream_id: session.workstream_id.clone(),
                    at_ms,
                };
                if db.record_doc_agent(&row)? {
                    changed.push(row.doc_id);
                }
            }
            Ok(changed)
        })
        .await?;
        self.announce(changed);
        Ok(())
    }

    /// Attributes stored history from the last rowid mark on, so rows an
    /// older server wrote are picked up on the next start.
    pub(crate) async fn backfill(&self) {
        if let Err(e) = self.try_backfill().await {
            tracing::warn!("doc attribution backfill stopped: {e}");
        }
    }

    async fn try_backfill(&self) -> Result<()> {
        let roots = thoughts_roots(&self.config.load().await?.config);
        // Leaves the mark alone, so history from before thoughts were set up is still attributed.
        if roots.is_empty() {
            return Ok(());
        }
        let db = self.db.clone();
        let (roots, after, upto) = blocking(move || {
            Ok((
                Arc::new(canon_roots(&roots)),
                db.agent_backfill_mark()?,
                db.max_session_event_rowid()?,
            ))
        })
        .await?;

        let mut changed: BTreeSet<String> = BTreeSet::new();
        let mut lo = after;
        while lo < upto {
            let hi = (lo + BACKFILL_WINDOW).min(upto);
            let (db, roots) = (self.db.clone(), roots.clone());
            // Sequential `Db` calls, each taking the mutex itself, so session actors interleave.
            let window = blocking(move || backfill_window(&db, &roots, lo, hi)).await?;
            changed.extend(window);
            lo = hi;
        }
        self.announce(changed.into_iter().collect());
        Ok(())
    }

    fn announce(&self, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }
        let _ = self.events.send(RuntimeEvent::DocStateChanged {
            ids,
            change: DocStateChange::Agent,
        });
    }
}

fn backfill_window(db: &Db, roots: &[RootCanon], lo: i64, hi: i64) -> Result<Vec<String>> {
    let uses = db.write_tool_uses_between(lo, hi)?;
    let mut changed = Vec::new();
    if !uses.is_empty() {
        let sessions: Vec<String> = uses
            .iter()
            .map(|u| u.session_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let ok = db.ok_tool_result_ids(&sessions, lo)?;
        for write in &uses {
            if !ok.contains(&(write.session_id.clone(), write.tool_use_id.clone())) {
                continue;
            }
            for doc_id in doc_ids(roots, &write.cwd, &write.paths) {
                let row = DocAgentRow {
                    doc_id,
                    session_id: write.session_id.clone(),
                    harness: write.harness,
                    workstream_id: write.workstream_id.clone(),
                    at_ms: write.created_at,
                };
                if db.record_doc_agent(&row)? {
                    changed.push(row.doc_id);
                }
            }
        }
    }
    db.set_agent_backfill_mark(hi)?;
    Ok(changed)
}
