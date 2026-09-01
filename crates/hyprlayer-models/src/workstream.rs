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
}

impl WorktreeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            WorktreeMode::Isolated => "isolated",
            WorktreeMode::Existing => "existing",
            WorktreeMode::Inplace => "inplace",
            WorktreeMode::Folder => "folder",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "isolated" => WorktreeMode::Isolated,
            "existing" => WorktreeMode::Existing,
            "folder" => WorktreeMode::Folder,
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
    pub pr_number: Option<u32>,
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

    pub fn shares_project_dir(&self) -> bool {
        matches!(
            self.worktree_mode,
            WorktreeMode::Inplace | WorktreeMode::Folder
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
                WorktreeMode::Inplace | WorktreeMode::Existing
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
