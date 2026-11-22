use serde::{Deserialize, Serialize};

use crate::agent::Stage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum WorktreeMode {
    Isolated,
    Existing,
    Inplace,
    Folder,
    // A worktree made outside this app; its folder is never moved or deleted.
    Attached,
}

impl WorktreeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            WorktreeMode::Isolated => "isolated",
            WorktreeMode::Existing => "existing",
            WorktreeMode::Inplace => "inplace",
            WorktreeMode::Folder => "folder",
            WorktreeMode::Attached => "attached",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "isolated" => WorktreeMode::Isolated,
            "existing" => WorktreeMode::Existing,
            "folder" => WorktreeMode::Folder,
            "attached" => WorktreeMode::Attached,
            _ => WorktreeMode::Inplace,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum WorkstreamStatus {
    Active,
    Archived,
}

impl WorkstreamStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkstreamStatus::Active => "active",
            WorkstreamStatus::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "archived" => WorkstreamStatus::Archived,
            _ => WorkstreamStatus::Active,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

impl PrState {
    pub fn as_str(self) -> &'static str {
        match self {
            PrState::Open => "open",
            PrState::Merged => "merged",
            PrState::Closed => "closed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(PrState::Open),
            "merged" => Some(PrState::Merged),
            "closed" => Some(PrState::Closed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ObservedPr {
    pub number: u32,
    pub state: PrState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Workstream {
    pub id: String,
    pub project_path: String,
    pub title: String,
    pub slug: String,
    pub branch: String,
    pub worktree_path: String,
    pub base_branch: String,
    pub worktree_mode: WorktreeMode,
    pub stage: Stage,
    pub status: WorkstreamStatus,
    /// Set only by attaching a PR; lookups then stop following the worktree's HEAD.
    pub pr_number: Option<u32>,
    /// What the PR watcher last saw for the current branch or attached PR.
    #[serde(default)]
    pub observed_pr: Option<ObservedPr>,
    /// The ref that observation came from, kept even when it found no PR.
    #[serde(default)]
    pub observed_ref: Option<String>,
    #[serde(skip)]
    pub observed_pr_head: Option<String>,
    #[serde(skip)]
    pub observed_pr_tip: Option<String>,
    pub created_at: f64,
    pub updated_at: f64,
    pub archived_at: Option<f64>,
}

impl Workstream {
    pub fn pr_ref(&self) -> String {
        match self.pr_number {
            Some(n) => n.to_string(),
            None => self.branch.clone(),
        }
    }

    pub fn is_inplace(&self) -> bool {
        self.worktree_mode == WorktreeMode::Inplace
    }

    /// Only these folders may be archived, restored or deleted by the app.
    pub fn app_owns_worktree(&self) -> bool {
        matches!(
            self.worktree_mode,
            WorktreeMode::Isolated | WorktreeMode::Existing
        )
    }

    pub fn is_folder(&self) -> bool {
        self.worktree_mode == WorktreeMode::Folder
    }

    pub fn app_owns_branch(&self) -> bool {
        self.worktree_mode == WorktreeMode::Isolated
    }

    pub fn follows_head(&self) -> bool {
        self.status == WorkstreamStatus::Active
            && matches!(
                self.worktree_mode,
                WorktreeMode::Inplace | WorktreeMode::Existing | WorktreeMode::Attached
            )
    }

    pub fn branch_to_adopt<'a>(&self, head: Option<&'a str>) -> Option<&'a str> {
        let head = head?;
        (self.follows_head() && !head.is_empty() && head != self.branch).then_some(head)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamDoc {
    pub workstream_id: String,
    pub doc_path: String,
    pub role: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workstream(mode: WorktreeMode, status: WorkstreamStatus) -> Workstream {
        Workstream {
            id: "w1".into(),
            project_path: "/p".into(),
            title: "t".into(),
            slug: "t-1".into(),
            branch: "hyprlayer/t-1".into(),
            worktree_path: "/wt".into(),
            base_branch: "main".into(),
            worktree_mode: mode,
            stage: Stage::General,
            status,
            pr_number: None,
            observed_pr: None,
            observed_ref: None,
            observed_pr_head: None,
            observed_pr_tip: None,
            created_at: 0.0,
            updated_at: 0.0,
            archived_at: None,
        }
    }

    #[test]
    fn worktree_mode_round_trips_and_falls_back_to_inplace() {
        for mode in [
            WorktreeMode::Isolated,
            WorktreeMode::Existing,
            WorktreeMode::Inplace,
            WorktreeMode::Folder,
            WorktreeMode::Attached,
        ] {
            assert_eq!(WorktreeMode::parse(mode.as_str()), mode);
        }
        assert_eq!(WorktreeMode::parse("unknown"), WorktreeMode::Inplace);
    }

    #[test]
    fn only_isolated_worktrees_are_app_owned() {
        for mode in [
            WorktreeMode::Existing,
            WorktreeMode::Inplace,
            WorktreeMode::Folder,
            WorktreeMode::Attached,
        ] {
            assert!(!workstream(mode, WorkstreamStatus::Active).app_owns_branch());
        }
        assert!(workstream(WorktreeMode::Isolated, WorkstreamStatus::Active).app_owns_branch());
    }

    #[test]
    fn an_archived_workstream_never_follows_head() {
        let ws = workstream(WorktreeMode::Inplace, WorkstreamStatus::Archived);
        assert!(!ws.follows_head());
        assert_eq!(ws.branch_to_adopt(Some("main")), None);
    }

    #[test]
    fn branch_to_adopt_ignores_empty_and_identical_heads() {
        let ws = workstream(WorktreeMode::Inplace, WorkstreamStatus::Active);
        assert_eq!(ws.branch_to_adopt(Some("main")), Some("main"));
        assert_eq!(ws.branch_to_adopt(Some("")), None);
        assert_eq!(ws.branch_to_adopt(Some("hyprlayer/t-1")), None);
        assert_eq!(ws.branch_to_adopt(None), None);
    }

    #[test]
    fn pr_ref_falls_back_to_the_branch() {
        let mut ws = workstream(WorktreeMode::Isolated, WorkstreamStatus::Active);
        assert_eq!(ws.pr_ref(), "hyprlayer/t-1");
        ws.pr_number = Some(42);
        assert_eq!(ws.pr_ref(), "42");
    }
}
