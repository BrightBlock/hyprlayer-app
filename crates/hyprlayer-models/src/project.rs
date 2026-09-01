use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum ProjectSource {
    Clone,
    Disk,
    Folder,
}

impl ProjectSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectSource::Clone => "clone",
            ProjectSource::Disk => "disk",
            ProjectSource::Folder => "folder",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "clone" => ProjectSource::Clone,
            "folder" => ProjectSource::Folder,
            _ => ProjectSource::Disk,
        }
    }

    pub fn is_git(self) -> bool {
        self != ProjectSource::Folder
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    pub path: String,
    pub name: String,
    pub origin_url: Option<String>,
    pub source: ProjectSource,
    pub created_at: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum CreateStep {
    Folder,
    Repository,
    Commit,
    GithubRepo,
    Push,
    Thoughts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum StepStatus {
    Running,
    Done,
    Failed,
    Skipped,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_source_round_trips_and_folders_are_not_git() {
        for source in [
            ProjectSource::Clone,
            ProjectSource::Disk,
            ProjectSource::Folder,
        ] {
            assert_eq!(ProjectSource::parse(source.as_str()), source);
        }
        assert_eq!(ProjectSource::parse("unknown"), ProjectSource::Disk);
        assert!(!ProjectSource::Folder.is_git());
        assert!(ProjectSource::Clone.is_git());
    }
}
