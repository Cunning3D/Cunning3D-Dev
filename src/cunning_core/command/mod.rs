use crate::nodes::{NodeGraph, NodeId};
use std::any::Any;
use std::fmt::Debug;

pub mod basic;

pub trait AsAny {
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any + Command> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub trait Command: AsAny + Debug + Send + Sync {
    fn apply(&mut self, graph: &mut NodeGraph);
    fn revert(&mut self, graph: &mut NodeGraph);
    fn merge(&mut self, _other: &dyn Command) -> bool {
        false
    }
    fn name(&self) -> &str;
}

#[derive(Debug)]
pub struct StackEntry {
    pub cmd: Box<dyn Command>,
    pub path: Vec<NodeId>,
}

#[derive(Debug, Default)]
pub struct UndoStack {
    undo: Vec<StackEntry>,
    redo: Vec<StackEntry>,
    max_depth: usize,
}

impl Clone for UndoStack {
    fn clone(&self) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            max_depth: self.max_depth,
        }
    }
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            max_depth: 100,
        }
    }

    pub fn push(&mut self, mut cmd: Box<dyn Command>, path: Vec<NodeId>) {
        // Try to merge with top of stack if paths match
        if let Some(top) = self.undo.last_mut() {
            if top.path == path {
                if top.cmd.merge(cmd.as_ref()) {
                    return;
                }
            }
        }

        self.redo.clear();
        self.undo.push(StackEntry { cmd, path });
        if self.undo.len() > self.max_depth {
            self.undo.remove(0);
        }
    }

    pub fn pop_undo(&mut self) -> Option<StackEntry> {
        self.undo.pop()
    }

    pub fn push_redo(&mut self, entry: StackEntry) {
        self.redo.push(entry);
    }

    pub fn pop_redo(&mut self) -> Option<StackEntry> {
        self.redo.pop()
    }

    pub fn push_undo(&mut self, entry: StackEntry) {
        self.undo.push(entry);
    }
}
