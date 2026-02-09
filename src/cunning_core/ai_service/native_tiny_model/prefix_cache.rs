use bevy::prelude::*;
use candle_core::Tensor;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Snapshot of the KV Cache at a specific point in the token sequence.
#[derive(Clone)]
pub struct KvCacheSnapshot {
    /// The KV tensors for each layer.
    /// Outer Vec: Layers
    /// Inner Tuple: (K, V)
    /// We use Arc<Tensor> to allow cheap cloning (candle Tensors are already RefCounted under the hood, but explicit Arc makes it clear we share data).
    /// Actually candle::Tensor IS an Arc wrapper, so simple Clone is cheap.
    pub layers: Vec<Option<(Tensor, Tensor)>>,
    pub token_count: usize,
}

/// A node in the Prefix Trie.
struct TrieNode {
    children: HashMap<u32, Box<TrieNode>>,
    /// If this node represents a valid cut-off point where we saved a snapshot.
    /// We don't save snapshots at every token to save memory/overhead.
    snapshot: Option<KvCacheSnapshot>,
    last_access: Instant,
}

impl TrieNode {
    fn new() -> Self {
        Self {
            children: HashMap::new(),
            snapshot: None,
            last_access: Instant::now(),
        }
    }
}

/// Manages prefix-based cache for LLM inference.
/// Allows reusing computation for common prefixes (e.g. System Prompt, Node Descriptions).
pub struct PrefixCache {
    root: TrieNode,
    max_snapshots: usize,
    /// Threshold: save a snapshot every N tokens
    snapshot_interval: usize,
}

impl PrefixCache {
    pub fn new() -> Self {
        Self {
            root: TrieNode::new(),
            max_snapshots: 20,     // Keep limited number of cached states
            snapshot_interval: 10, // Snapshot every 10 tokens? Or maybe at semantic boundaries?
        }
    }

    /// Find the longest matching prefix in the cache.
    /// Returns: (Matched Token Count, Snapshot)
    pub fn find_longest_prefix(&mut self, tokens: &[u32]) -> Option<(usize, KvCacheSnapshot)> {
        let mut node = &mut self.root;
        let mut last_snapshot = None;
        let mut depth = 0;

        for token in tokens {
            if let Some(child) = node.children.get_mut(token) {
                node = child;
                depth += 1;
                node.last_access = Instant::now();
                if let Some(snap) = &node.snapshot {
                    last_snapshot = Some((depth, snap.clone()));
                }
            } else {
                break;
            }
        }

        last_snapshot
    }

    /// Insert a new snapshot into the tree at the given token path.
    pub fn insert_snapshot(&mut self, tokens: &[u32], snapshot: KvCacheSnapshot) {
        let mut node = &mut self.root;
        for token in tokens {
            node = node
                .children
                .entry(*token)
                .or_insert_with(|| Box::new(TrieNode::new()));
        }
        node.snapshot = Some(snapshot);
        node.last_access = Instant::now();

        // TODO: Prune old snapshots if limit reached
    }
}
