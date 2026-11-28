//! Which cached reads a runtime event invalidates; the table `src/lib/events.ts` encodes in its listeners.

use hyprlayer_core::models::RuntimeEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Topic {
    Bootstrap,
    Workstreams,
    ActiveSessions,
}

pub fn topics_for(event: &RuntimeEvent) -> &'static [Topic] {
    use RuntimeEvent as E;
    match event {
        E::ConfigChanged | E::BinariesChanged | E::ProjectsChanged => &[Topic::Bootstrap],
        E::WorkstreamsChanged { .. } | E::WorkstreamBranchChanged { .. } => &[Topic::Workstreams],
        // CI churn would recompute git status for every worktree; only an observed change re-reads the lists.
        E::PrStatusChanged {
            observed_pr_changed: true,
            ..
        } => &[Topic::Workstreams],
        E::SessionLifecycle { .. } => &[Topic::ActiveSessions],
        _ => &[],
    }
}
