use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use super::indexer::{DocMeta, ThoughtQuery, ThoughtsIndex};
use super::search;
use crate::docs::vocab::{DocStatus, DocType};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DocCounts {
    pub total: u32,
    pub types: Vec<TypeCount>,
    pub statuses: Vec<StatusCount>,
    /// Count desc, then name; ignores the project scope so a picker can list every project.
    pub projects: Vec<NameCount>,
    /// Name asc.
    pub authors: Vec<NameCount>,
    pub unread: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct TypeCount {
    pub doc_type: DocType,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StatusCount {
    pub status: DocStatus,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct NameCount {
    pub name: String,
    pub count: u32,
}

impl ThoughtsIndex {
    /// Each mailbox counts the list it opens: `filters` apply except on the box's own
    /// dimension, and text never does (no body reads). Projects and authors ignore
    /// `filters` so their pickers keep every option. Unknown types and statuses are not counted.
    pub fn counts(
        &self,
        project: Option<&str>,
        filters: &ThoughtQuery,
        is_unread: impl Fn(&DocMeta, u64) -> bool,
    ) -> DocCounts {
        let scope = ThoughtQuery {
            projects: project.map(str::to_string).into_iter().collect(),
            ..Default::default()
        };
        let filtered = ThoughtQuery {
            projects: scope.projects.clone(),
            ..filters.clone()
        };
        let any_type = ThoughtQuery {
            types: Vec::new(),
            ..filtered.clone()
        };
        let any_status = ThoughtQuery {
            statuses: Vec::new(),
            ..filtered.clone()
        };
        let mut total = 0u32;
        let mut unread = 0u32;
        let mut types: HashMap<DocType, u32> = HashMap::new();
        let mut statuses: HashMap<DocStatus, u32> = HashMap::new();
        let mut projects: HashMap<String, u32> = HashMap::new();
        let mut authors: BTreeMap<String, u32> = BTreeMap::new();

        let map = self.records.read().expect("index lock poisoned");
        for record in map.values() {
            let meta = &record.meta;
            if let Some(name) = meta.project_key() {
                *projects.entry(name.to_string()).or_default() += 1;
            }
            if !search::passes_filters(&scope, meta) {
                continue;
            }
            if let Some(author) = meta
                .fm
                .author
                .as_deref()
                .map(str::trim)
                .filter(|a| !a.is_empty())
            {
                *authors.entry(author.to_string()).or_default() += 1;
            }
            if search::passes_filters(&any_type, meta) {
                if let Some(t) = meta.fm.doc_type.as_deref().and_then(DocType::try_parse) {
                    *types.entry(t).or_default() += 1;
                }
            }
            if search::passes_filters(&any_status, meta) {
                if let Some(s) = meta.fm.status.as_deref().and_then(DocStatus::try_parse) {
                    *statuses.entry(s).or_default() += 1;
                }
            }
            if !search::passes_filters(&filtered, meta) {
                continue;
            }
            total += 1;
            if is_unread(meta, record.content_hash) {
                unread += 1;
            }
        }
        drop(map);

        let mut projects: Vec<NameCount> = projects
            .into_iter()
            .map(|(name, count)| NameCount { name, count })
            .collect();
        projects.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));

        DocCounts {
            total,
            types: DocType::ALL
                .into_iter()
                .map(|doc_type| TypeCount {
                    doc_type,
                    count: types.get(&doc_type).copied().unwrap_or(0),
                })
                .collect(),
            statuses: DocStatus::ALL
                .into_iter()
                .map(|status| StatusCount {
                    status,
                    count: statuses.get(&status).copied().unwrap_or(0),
                })
                .collect(),
            projects,
            authors: authors
                .into_iter()
                .map(|(name, count)| NameCount { name, count })
                .collect(),
            unread,
        }
    }
}
