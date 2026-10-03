use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Typechange,
    Other,
}

impl FileChangeKind {
    pub fn try_parse(s: &str) -> Option<FileChangeKind> {
        match s {
            "added" | "untracked" => Some(FileChangeKind::Added),
            "modified" => Some(FileChangeKind::Modified),
            "deleted" => Some(FileChangeKind::Deleted),
            "renamed" => Some(FileChangeKind::Renamed),
            "typechange" => Some(FileChangeKind::Typechange),
            "other" => Some(FileChangeKind::Other),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> FileChangeKind {
        FileChangeKind::try_parse(s).unwrap_or(FileChangeKind::Other)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            FileChangeKind::Added => "added",
            FileChangeKind::Modified => "modified",
            FileChangeKind::Deleted => "deleted",
            FileChangeKind::Renamed => "renamed",
            FileChangeKind::Typechange => "typechange",
            FileChangeKind::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    InProgress,
    ChangesRequested,
    Approved,
}

impl ReviewStatus {
    pub fn try_parse(s: &str) -> Option<ReviewStatus> {
        match s {
            "in_progress" => Some(ReviewStatus::InProgress),
            "changes_requested" => Some(ReviewStatus::ChangesRequested),
            "approved" => Some(ReviewStatus::Approved),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> ReviewStatus {
        ReviewStatus::try_parse(s).unwrap_or(ReviewStatus::InProgress)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ReviewStatus::InProgress => "in_progress",
            ReviewStatus::ChangesRequested => "changes_requested",
            ReviewStatus::Approved => "approved",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Comment,
    RequestChanges,
    Approve,
}

impl ReviewVerdict {
    pub fn try_parse(s: &str) -> Option<ReviewVerdict> {
        match s {
            "comment" => Some(ReviewVerdict::Comment),
            "request_changes" => Some(ReviewVerdict::RequestChanges),
            "approve" => Some(ReviewVerdict::Approve),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> ReviewVerdict {
        ReviewVerdict::try_parse(s).unwrap_or(ReviewVerdict::Comment)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ReviewVerdict::Comment => "comment",
            ReviewVerdict::RequestChanges => "request_changes",
            ReviewVerdict::Approve => "approve",
        }
    }
}

/// Resolution is not a state: a sent comment can be resolved and keep its history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ReviewCommentState {
    Pending,
    Saved,
    Sending,
    Sent,
}

impl ReviewCommentState {
    pub fn try_parse(s: &str) -> Option<ReviewCommentState> {
        match s {
            "pending" => Some(ReviewCommentState::Pending),
            "saved" => Some(ReviewCommentState::Saved),
            "sending" => Some(ReviewCommentState::Sending),
            "sent" => Some(ReviewCommentState::Sent),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> ReviewCommentState {
        ReviewCommentState::try_parse(s).unwrap_or(ReviewCommentState::Pending)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ReviewCommentState::Pending => "pending",
            ReviewCommentState::Saved => "saved",
            ReviewCommentState::Sending => "sending",
            ReviewCommentState::Sent => "sent",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ReviewNoteKind {
    Note,
    NeedsYourCall,
    OutsidePlanScope,
}

impl ReviewNoteKind {
    pub fn try_parse(s: &str) -> Option<ReviewNoteKind> {
        match s {
            "note" => Some(ReviewNoteKind::Note),
            "needs_your_call" => Some(ReviewNoteKind::NeedsYourCall),
            "outside_plan_scope" => Some(ReviewNoteKind::OutsidePlanScope),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> ReviewNoteKind {
        ReviewNoteKind::try_parse(s).unwrap_or(ReviewNoteKind::Note)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ReviewNoteKind::Note => "note",
            ReviewNoteKind::NeedsYourCall => "needs_your_call",
            ReviewNoteKind::OutsidePlanScope => "outside_plan_scope",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ReviewSide {
    Original,
    Modified,
}

impl ReviewSide {
    pub fn try_parse(s: &str) -> Option<ReviewSide> {
        match s {
            "original" => Some(ReviewSide::Original),
            "modified" => Some(ReviewSide::Modified),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> ReviewSide {
        ReviewSide::try_parse(s).unwrap_or(ReviewSide::Modified)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ReviewSide::Original => "original",
            ReviewSide::Modified => "modified",
        }
    }
}
