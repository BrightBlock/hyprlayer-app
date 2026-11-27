//! The session transcript reducer: a pure port of the webview's `reduceEvent`, `mergeEntries` and ask handling.

pub mod replay;
pub mod sync;
mod text;
pub mod transcript;

pub use sync::{closed_turn_in, merge_entries, LastTurn, Merged};
pub use transcript::{
    add_user_message, cancel_pending_prompts, reduce_event, resolve_permission, resolve_question,
    restored_view, settle_unwaited_asks, ItemKind, PendingQuestion, PermissionDecision,
    SessionView, TranscriptItem,
};
