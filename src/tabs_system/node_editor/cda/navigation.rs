//! CDA navigation: breadcrumb + double-click enter/return
use super::editor_state::CDAEditorState;
use crate::cunning_core::cda::library::global_cda_library;
use crate::cunning_core::cda::CdaAssetRef;
use crate::nodes::structs::{NodeGraph, NodeId, NodeType};
use bevy_egui::egui::Vec2;
use std::collections::HashSet;

const MAX_CDA_DEPTH: usize = 64;

fn graph_by_uuid_chain<'a>(
    root: &'a NodeGraph,
    defs: &'a std::collections::HashMap<uuid::Uuid, crate::cunning_core::cda::CDAAsset>,
    chain: &[uuid::Uuid],
) -> Option<&'a NodeGraph> {
    let mut cur = root;
    for u in chain {
        cur = &defs.get(u)?.inner_graph;
    }
    Some(cur)
}

fn graph_by_uuid_chain_mut(
    root: *mut NodeGraph,
    defs: &mut std::collections::HashMap<uuid::Uuid, crate::cunning_core::cda::CDAAsset>,
    chain: &[uuid::Uuid],
) -> Option<*mut NodeGraph> {
    let mut cur = root;
    for u in chain {
        cur = &mut defs.get_mut(u)?.inner_graph as *mut _;
    }
    Some(cur)
}

/// Handle double-click to enter CDA, returns new (pan, zoom) if successful
pub fn handle_double_click_enter(
    node_id: NodeId,
    graph: &NodeGraph,
    cda_state: &mut CDAEditorState,
    pan: Vec2,
    zoom: f32,
    canvas_size: Vec2,
) -> Option<(Vec2, f32)> {
    if let Some(node) = graph.nodes.get(&node_id) {
        if let NodeType::CDA(data) = &node.node_type {
            cda_state.enter_cda(node_id, pan, zoom);

            if let Some(lib) = global_cda_library() {
                if let Some(g) = lib.def_guard(data.asset_ref.uuid) {
                    if let (Some(center), Some(z)) = (g.asset().view_center, g.asset().view_zoom) {
                        let world_center = Vec2::new(center[0], center[1]);
                        let new_zoom = z;
                        let new_pan = canvas_size * 0.5 - world_center * new_zoom;
                        return Some((new_pan, new_zoom));
                    }
                }
            }

            return Some((Vec2::ZERO, 1.0));
        }
    }
    None
}

fn asset_ref_from_node(g: &NodeGraph, node_id: NodeId) -> Option<CdaAssetRef> {
    let n = g.nodes.get(&node_id)?;
    match &n.node_type {
        NodeType::CDA(d) => Some(d.asset_ref.clone()),
        _ => None,
    }
}

/// Resolve the current CDA asset uuid for a breadcrumb path (deepest CDA entered).
pub fn current_cda_uuid_by_path(root: &NodeGraph, path: &[NodeId]) -> Option<uuid::Uuid> {
    if path.is_empty() {
        return None;
    }
    let lib = global_cda_library()?;
    lib.with_defs_mut(|defs| {
        let mut cur: *const NodeGraph = root as *const _;
        let mut seen: HashSet<NodeId> = HashSet::new();
        let mut out: Option<uuid::Uuid> = None;
        for (depth, id) in path.iter().copied().enumerate() {
            if depth >= MAX_CDA_DEPTH || !seen.insert(id) {
                break;
            }
            unsafe {
                let g = &*cur;
                let r = asset_ref_from_node(g, id)?;
                out = Some(r.uuid);
                let a = defs.get(&r.uuid)?;
                cur = &a.inner_graph as *const _;
            }
        }
        out
    })?
}

pub fn with_graph_by_path_mut<R>(
    root: &mut NodeGraph,
    path: &[NodeId],
    f: impl FnOnce(&mut NodeGraph) -> R,
) -> R {
    if path.is_empty() {
        return f(root);
    }
    let Some(lib) = global_cda_library() else {
        return f(root);
    };

    let mut f = Some(f);
    let out = lib.with_defs_mut(|defs| {
        let mut chain: Vec<uuid::Uuid> = Vec::new();
        let mut seen: HashSet<NodeId> = HashSet::new();
        for (depth, id) in path.iter().copied().enumerate() {
            if depth >= MAX_CDA_DEPTH || !seen.insert(id) {
                break;
            }
            let r = {
                let defs_ro: &std::collections::HashMap<
                    uuid::Uuid,
                    crate::cunning_core::cda::CDAAsset,
                > = &*defs;
                let g = graph_by_uuid_chain(root, defs_ro, &chain).unwrap_or(root);
                asset_ref_from_node(g, id)
            }?;
            if !defs.contains_key(&r.uuid) {
                if r.path.is_empty() {
                    return None;
                }
                let a = crate::cunning_core::cda::CDAAsset::load_dcc(&r.path).ok()?;
                defs.insert(a.id, a);
            }
            chain.push(r.uuid);
        }
        let cur = graph_by_uuid_chain_mut(root as *mut _, defs, &chain)?;
        Some(unsafe { f.take().unwrap()(&mut *cur) })
    });

    match out {
        Some(Some(v)) => v,
        _ => f.take().unwrap()(root),
    }
}

pub fn with_graph_by_path<R>(
    root: &NodeGraph,
    path: &[NodeId],
    f: impl FnOnce(&NodeGraph) -> R,
) -> R {
    if path.is_empty() {
        return f(root);
    }
    let Some(lib) = global_cda_library() else {
        return f(root);
    };

    let mut f = Some(f);
    let mut out: Option<R> = None;

    let ok = lib
        .with_defs_mut(|defs| {
            let mut cur: *const NodeGraph = root as *const _;
            let mut seen: HashSet<NodeId> = HashSet::new();
            for (depth, id) in path.iter().copied().enumerate() {
                if depth >= MAX_CDA_DEPTH || !seen.insert(id) {
                    break;
                }
                unsafe {
                    let g = &*cur;
                    let Some(r) = asset_ref_from_node(g, id) else {
                        break;
                    };
                    let Some(a) = defs.get(&r.uuid) else {
                        break;
                    };
                    cur = &a.inner_graph as *const _;
                }
            }
            out = Some(unsafe { f.take().unwrap()(&*cur) });
        })
        .is_some();

    if ok {
        out.unwrap()
    } else {
        f.take().unwrap()(root)
    }
}

pub fn graph_snapshot_by_path(root: &NodeGraph, path: &[NodeId]) -> NodeGraph {
    with_graph_by_path(root, path, |g| g.clone())
}

pub fn with_current_graph_mut<R>(
    root: &mut NodeGraph,
    cda_state: &CDAEditorState,
    f: impl FnOnce(&mut NodeGraph) -> R,
) -> R {
    with_graph_by_path_mut(root, &cda_state.breadcrumb(), f)
}

pub fn with_current_graph<R>(
    root: &NodeGraph,
    cda_state: &CDAEditorState,
    f: impl FnOnce(&NodeGraph) -> R,
) -> R {
    with_graph_by_path(root, &cda_state.breadcrumb(), f)
}

pub fn sync_current_cda_input_titles(root: &mut NodeGraph, cda_state: &CDAEditorState) -> bool {
    let _ = (root, cda_state);
    // NOTE: Never write instance connection info back into definition.
    // Input titles derived from parent connections must be a pure UI overlay.
    false
}
