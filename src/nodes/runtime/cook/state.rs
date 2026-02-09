use crate::nodes::structs::NodeId;
use dashmap::{DashMap, DashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeCookState {
    Idle = 0,
    Queued = 1,
    Running = 2,
    Blocked = 3,
    Failed = 4,
}

impl NodeCookState {
    #[inline]
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

#[derive(Default, Debug)]
pub struct CookVizShared {
    pub cook_id: AtomicU64,
    pub active: AtomicBool,
    pub scope: DashSet<NodeId>,
    pub states: DashMap<NodeId, NodeCookState>,
    pub errors: DashMap<NodeId, String>,
}

impl CookVizShared {
    #[inline]
    pub fn begin(&self, cook_id: u64) {
        self.cook_id.store(cook_id, Ordering::Relaxed);
        self.active.store(true, Ordering::Relaxed);
        self.scope.clear();
        self.states.clear();
        self.errors.clear();
    }

    #[inline]
    pub fn end(&self) {
        self.active.store(false, Ordering::Relaxed);
    }

    #[inline]
    pub fn in_scope(&self, n: NodeId) -> bool {
        self.scope.contains(&n)
    }

    #[inline]
    pub fn set_scope<I: IntoIterator<Item = NodeId>>(&self, nodes: I) {
        for n in nodes {
            self.scope.insert(n);
        }
    }

    #[inline]
    pub fn set_state(&self, n: NodeId, s: NodeCookState) {
        self.states.insert(n, s);
        if s != NodeCookState::Failed {
            self.errors.remove(&n);
        }
    }

    #[inline]
    pub fn set_failed(&self, n: NodeId, msg: impl Into<String>) {
        self.states.insert(n, NodeCookState::Failed);
        self.errors.insert(n, msg.into());
    }
}

