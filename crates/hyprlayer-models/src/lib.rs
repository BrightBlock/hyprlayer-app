//! Dependency-light serialized contracts shared by `hyprlayer-core` and its
//! shells (ADR-0008).
//!
//! Every type here is supported API, re-exported at the crate root and reached
//! by core consumers as `hyprlayer_core::models`. `serde` is the only normal
//! dependency; `specta` is optional and derives the TypeScript bindings the
//! desktop shell generates.

pub mod agent;
pub mod agent_control;
pub mod goal;
pub mod project;
pub mod review;
pub mod runtime;
pub mod script;
pub mod session;
pub mod skill;
pub mod terminal;
pub mod workstream;

pub use agent::{Harness, PermissionMode, Stage};
pub use agent_control::{
    AgentCaller, AgentControlLevel, AgentSignal, AgentStatusReport, AgentWhoami, PtyAgent,
    PtyAgentContext, PtyAgentState, PtyAgentStatus, PtyControlOutcome, RevealTarget,
    SpawnPtyAgentRequest, SpawnSessionRequest, SpawnedAgent, UiRequest, WaitOutcome, WaitUntil,
};
pub use goal::{
    Goal, GoalBrief, GoalCheck, GoalCheckKind, GoalCheckOutcome, GoalCheckResult, GoalDetail,
    GoalEndReason, GoalIteration, GoalIterationNote, GoalLimits, GoalPauseReason, GoalSource,
    GoalSpec, GoalStatus, GoalStep,
};
pub use project::{CreateStep, ProjectRecord, ProjectSource, StepStatus};
pub use review::{
    FileChangeKind, ReviewCommentState, ReviewNoteKind, ReviewSide, ReviewStatus, ReviewVerdict,
};
pub use runtime::{DocStateChange, RuntimeEvent, ThoughtsIndexUpdateKind};
pub use script::{RunPhase, ScriptKind, ScriptRuntime, ScriptStatus};
pub use session::{
    ClientRef, ExitOutcome, LogCursor, Question, QuestionAnswers, QuestionOption, RevisedTurn,
    Session, SessionEvent, SessionLogEntry, SessionOrigin, SessionRev, SessionState,
    SessionStreamItem, SessionTurns, TurnKind, TurnStatus,
};
pub use skill::{SkillScope, SkillSourceKind, SkillTarget};
pub use terminal::{
    PtyFirstPrompt, PtyLaunchSession, PtyLaunched, Rgb, TerminalEvent, TerminalTheme,
};
pub use workstream::{Workstream, WorkstreamDoc, WorkstreamStatus, WorktreeMode};
