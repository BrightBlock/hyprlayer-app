use serde::{Deserialize, Serialize};

use crate::session::ClientRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Running,
    Paused,
    Met,
    Exhausted,
    Failed,
    Stopped,
}

impl GoalStatus {
    pub fn try_parse(s: &str) -> Option<GoalStatus> {
        match s {
            "running" => Some(GoalStatus::Running),
            "paused" => Some(GoalStatus::Paused),
            "met" => Some(GoalStatus::Met),
            "exhausted" => Some(GoalStatus::Exhausted),
            "failed" => Some(GoalStatus::Failed),
            "stopped" => Some(GoalStatus::Stopped),
            _ => None,
        }
    }

    /// Stopped: a server never drives a status it can't read.
    pub fn parse(s: &str) -> GoalStatus {
        GoalStatus::try_parse(s).unwrap_or(GoalStatus::Stopped)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            GoalStatus::Running => "running",
            GoalStatus::Paused => "paused",
            GoalStatus::Met => "met",
            GoalStatus::Exhausted => "exhausted",
            GoalStatus::Failed => "failed",
            GoalStatus::Stopped => "stopped",
        }
    }

    pub fn is_live(self) -> bool {
        matches!(self, GoalStatus::Running | GoalStatus::Paused)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum GoalStep {
    Working,
    Checking,
    Waiting,
    Resuming,
    Pausing,
    Stopping,
}

impl GoalStep {
    pub fn try_parse(s: &str) -> Option<GoalStep> {
        match s {
            "working" => Some(GoalStep::Working),
            "checking" => Some(GoalStep::Checking),
            "waiting" => Some(GoalStep::Waiting),
            "resuming" => Some(GoalStep::Resuming),
            "pausing" => Some(GoalStep::Pausing),
            "stopping" => Some(GoalStep::Stopping),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> GoalStep {
        GoalStep::try_parse(s).unwrap_or(GoalStep::Working)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            GoalStep::Working => "working",
            GoalStep::Checking => "checking",
            GoalStep::Waiting => "waiting",
            GoalStep::Resuming => "resuming",
            GoalStep::Pausing => "pausing",
            GoalStep::Stopping => "stopping",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum GoalPauseReason {
    User,
    NeedsHuman,
    TurnFailures,
    HarnessCrashes,
    RestartLoop,
    RelaunchFailed,
}

impl GoalPauseReason {
    pub fn try_parse(s: &str) -> Option<GoalPauseReason> {
        match s {
            "user" => Some(GoalPauseReason::User),
            "needs_human" => Some(GoalPauseReason::NeedsHuman),
            "turn_failures" => Some(GoalPauseReason::TurnFailures),
            "harness_crashes" => Some(GoalPauseReason::HarnessCrashes),
            "restart_loop" => Some(GoalPauseReason::RestartLoop),
            "relaunch_failed" => Some(GoalPauseReason::RelaunchFailed),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> GoalPauseReason {
        GoalPauseReason::try_parse(s).unwrap_or(GoalPauseReason::User)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            GoalPauseReason::User => "user",
            GoalPauseReason::NeedsHuman => "needs_human",
            GoalPauseReason::TurnFailures => "turn_failures",
            GoalPauseReason::HarnessCrashes => "harness_crashes",
            GoalPauseReason::RestartLoop => "restart_loop",
            GoalPauseReason::RelaunchFailed => "relaunch_failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum GoalEndReason {
    ChecksPassed,
    IterationLimit,
    BudgetLimit,
    TokenLimit,
    TimeLimit,
    Stalled,
    ChecksBroken,
    StoppedByUser,
    SessionEnded,
    WorkstreamArchived,
}

impl GoalEndReason {
    pub fn try_parse(s: &str) -> Option<GoalEndReason> {
        match s {
            "checks_passed" => Some(GoalEndReason::ChecksPassed),
            "iteration_limit" => Some(GoalEndReason::IterationLimit),
            "budget_limit" => Some(GoalEndReason::BudgetLimit),
            "token_limit" => Some(GoalEndReason::TokenLimit),
            "time_limit" => Some(GoalEndReason::TimeLimit),
            "stalled" => Some(GoalEndReason::Stalled),
            "checks_broken" => Some(GoalEndReason::ChecksBroken),
            "stopped_by_user" => Some(GoalEndReason::StoppedByUser),
            "session_ended" => Some(GoalEndReason::SessionEnded),
            "workstream_archived" => Some(GoalEndReason::WorkstreamArchived),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> GoalEndReason {
        GoalEndReason::try_parse(s).unwrap_or(GoalEndReason::StoppedByUser)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            GoalEndReason::ChecksPassed => "checks_passed",
            GoalEndReason::IterationLimit => "iteration_limit",
            GoalEndReason::BudgetLimit => "budget_limit",
            GoalEndReason::TokenLimit => "token_limit",
            GoalEndReason::TimeLimit => "time_limit",
            GoalEndReason::Stalled => "stalled",
            GoalEndReason::ChecksBroken => "checks_broken",
            GoalEndReason::StoppedByUser => "stopped_by_user",
            GoalEndReason::SessionEnded => "session_ended",
            GoalEndReason::WorkstreamArchived => "workstream_archived",
        }
    }

    pub fn status(self) -> GoalStatus {
        match self {
            GoalEndReason::ChecksPassed => GoalStatus::Met,
            GoalEndReason::IterationLimit
            | GoalEndReason::BudgetLimit
            | GoalEndReason::TokenLimit
            | GoalEndReason::TimeLimit => GoalStatus::Exhausted,
            GoalEndReason::Stalled | GoalEndReason::ChecksBroken => GoalStatus::Failed,
            GoalEndReason::StoppedByUser
            | GoalEndReason::SessionEnded
            | GoalEndReason::WorkstreamArchived => GoalStatus::Stopped,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum GoalSource {
    Composer,
    Plan,
    PlanPhase,
    Research,
    Validation,
}

impl GoalSource {
    pub fn try_parse(s: &str) -> Option<GoalSource> {
        match s {
            "composer" => Some(GoalSource::Composer),
            "plan" => Some(GoalSource::Plan),
            "plan_phase" => Some(GoalSource::PlanPhase),
            "research" => Some(GoalSource::Research),
            "validation" => Some(GoalSource::Validation),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> GoalSource {
        GoalSource::try_parse(s).unwrap_or(GoalSource::Composer)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            GoalSource::Composer => "composer",
            GoalSource::Plan => "plan",
            GoalSource::PlanPhase => "plan_phase",
            GoalSource::Research => "research",
            GoalSource::Validation => "validation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum GoalCheckOutcome {
    Pass,
    Fail,
    Error,
}

impl GoalCheckOutcome {
    pub fn try_parse(s: &str) -> Option<GoalCheckOutcome> {
        match s {
            "pass" => Some(GoalCheckOutcome::Pass),
            "fail" => Some(GoalCheckOutcome::Fail),
            "error" => Some(GoalCheckOutcome::Error),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> GoalCheckOutcome {
        GoalCheckOutcome::try_parse(s).unwrap_or(GoalCheckOutcome::Error)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            GoalCheckOutcome::Pass => "pass",
            GoalCheckOutcome::Fail => "fail",
            GoalCheckOutcome::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum GoalCheckKind {
    Command,
    PlanBoxes,
    Evaluator,
    Validation,
}

impl GoalCheckKind {
    pub fn try_parse(s: &str) -> Option<GoalCheckKind> {
        match s {
            "command" => Some(GoalCheckKind::Command),
            "plan_boxes" => Some(GoalCheckKind::PlanBoxes),
            "evaluator" => Some(GoalCheckKind::Evaluator),
            "validation" => Some(GoalCheckKind::Validation),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> GoalCheckKind {
        GoalCheckKind::try_parse(s).unwrap_or(GoalCheckKind::Command)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            GoalCheckKind::Command => "command",
            GoalCheckKind::PlanBoxes => "plan_boxes",
            GoalCheckKind::Evaluator => "evaluator",
            GoalCheckKind::Validation => "validation",
        }
    }
}

/// The column is nullable, so there is no `parse` fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum GoalIterationNote {
    CutOff,
    Steered,
    Interrupted,
}

impl GoalIterationNote {
    pub fn try_parse(s: &str) -> Option<GoalIterationNote> {
        match s {
            "cut_off" => Some(GoalIterationNote::CutOff),
            "steered" => Some(GoalIterationNote::Steered),
            "interrupted" => Some(GoalIterationNote::Interrupted),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            GoalIterationNote::CutOff => "cut_off",
            GoalIterationNote::Steered => "steered",
            GoalIterationNote::Interrupted => "interrupted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum GoalCheck {
    Command {
        command: String,
        timeout_secs: u32,
        #[serde(default)]
        label: Option<String>,
    },
    PlanBoxes {
        plan_path: String,
        #[serde(default)]
        phase: Option<String>,
    },
    Evaluator {
        #[serde(default)]
        model: Option<String>,
    },
    Validation {
        plan_path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GoalLimits {
    pub max_iterations: u32,
    pub max_cost_usd: Option<f64>,
    pub max_tokens: Option<u32>,
    pub max_active_secs: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GoalSpec {
    pub condition: String,
    pub work_prompt: String,
    pub source: GoalSource,
    pub source_ref: Option<String>,
    pub source_phase: Option<String>,
    pub checks: Vec<GoalCheck>,
    pub limits: GoalLimits,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    pub id: String,
    pub session_id: String,
    pub workstream_id: String,
    pub condition: String,
    pub work_prompt: String,
    pub source: GoalSource,
    pub source_ref: Option<String>,
    pub source_phase: Option<String>,
    pub checks: Vec<GoalCheck>,
    pub limits: GoalLimits,
    pub status: GoalStatus,
    pub step: Option<GoalStep>,
    pub pause_reason: Option<GoalPauseReason>,
    pub end_reason: Option<GoalEndReason>,
    pub iteration: u32,
    pub spent_cost_usd: f64,
    pub spent_tokens: u32,
    pub active_ms: u32,
    pub last_outcome: Option<GoalCheckOutcome>,
    pub last_reason: Option<String>,
    pub author: Option<ClientRef>,
    pub started_at: f64,
    pub updated_at: f64,
    pub ended_at: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GoalCheckResult {
    pub kind: GoalCheckKind,
    pub label: String,
    pub outcome: GoalCheckOutcome,
    pub exit_code: Option<i32>,
    pub duration_ms: u32,
    /// Kept for fail and error only, at most 8 KiB.
    pub output_tail: String,
    pub reason: Option<String>,
    /// The Validation check's validate session.
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GoalIteration {
    pub goal_id: String,
    pub n: u32,
    pub client_message_id: String,
    pub started_at: f64,
    pub ended_at: Option<f64>,
    pub turn_status: Option<String>,
    pub cost_usd: Option<f64>,
    pub tokens: Option<u32>,
    pub outcome: Option<GoalCheckOutcome>,
    pub reason: Option<String>,
    pub results: Vec<GoalCheckResult>,
    /// Hex of a 64-bit hash of HEAD's oid and the worktree diff; a u64 cannot cross specta.
    pub progress_hash: Option<String>,
    pub note: Option<GoalIterationNote>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GoalDetail {
    pub goal: Goal,
    pub iterations: Vec<GoalIteration>,
}

/// `pause_reason` is here so a board can tell a user pause from a goal that needs you.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GoalBrief {
    pub id: String,
    pub status: GoalStatus,
    pub step: Option<GoalStep>,
    pub pause_reason: Option<GoalPauseReason>,
    pub iteration: u32,
    pub max_iterations: u32,
}
