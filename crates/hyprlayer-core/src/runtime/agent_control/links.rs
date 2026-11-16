use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use hyprlayer_models::AgentCaller;

/// Spawn lineage no session row can hold: anything a terminal spawned, and terminals a session spawned.
/// In memory only, since PTYs die with the server; a session's link lasts until its row is deleted.
#[derive(Default)]
pub(crate) struct AgentLinks {
    by_child: Mutex<HashMap<String, Link>>,
}

#[derive(Clone)]
pub(crate) struct Link {
    pub parent: AgentCaller,
    /// The child's own depth.
    pub depth: u32,
}

impl AgentLinks {
    pub(crate) fn link(&self, child_id: &str, parent: AgentCaller, depth: u32) {
        self.lock()
            .insert(child_id.to_string(), Link { parent, depth });
    }

    /// The child is gone: its PTY ended, or its session row was deleted.
    pub(crate) fn unlink(&self, child_id: &str) {
        self.lock().remove(child_id);
    }

    pub(crate) fn get(&self, child_id: &str) -> Option<Link> {
        self.lock().get(child_id).cloned()
    }

    /// Every child linked to `parent_id`, exited sessions included.
    pub(crate) fn children_of(&self, parent_id: &str) -> Vec<String> {
        self.lock()
            .iter()
            .filter(|(_, l)| l.parent.id() == parent_id)
            .map(|(child, _)| child.clone())
            .collect()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Link>> {
        self.by_child.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
