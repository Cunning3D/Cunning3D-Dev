//! Shared viewport-embedded coverlay dock types and layout helpers.
use egui_dock::{DockState, Node, NodeIndex, Tree};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CoverlayPanelKind {
    Manager,
    VoxelTools,
    VoxelPalette,
    SdfTools,
    VoxelDebug,
    Import,
    Export,
    Anim,
    NodeCoverlay,
    Parameters,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CoverlayPanelKey {
    DirectVoxel { node_id: Uuid, kind: CoverlayPanelKind },
    DirectNode { node_id: Uuid },
    CdaManager { inst_id: Uuid },
    CdaUnit { inst_id: Uuid, unit_id: Uuid },
    CdaVoxel { inst_id: Uuid, internal_id: Uuid, kind: CoverlayPanelKind },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CoverlayDockPanel {
    pub key: CoverlayPanelKey,
    pub title: String,
    pub kind: CoverlayPanelKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ViewportDockTab {
    Viewport,
    Coverlay(CoverlayDockPanel),
}

pub const VIEWPORT_DOCK_LAYOUT_KEY: &str = "viewport_coverlay_dock_layout_json";
pub const VIEWPORT_DOCK_PRESET_KEY: &str = "viewport_coverlay_dock_preset_json";
// Keep a large viewport, but don't crush tool panels (desktop + wasm).
pub const VIEWPORT_KEEP_X: f32 = 0.82;
pub const VIEWPORT_KEEP_Y: f32 = 0.80;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DockSlot {
    Left,
    Right,
    Top,
    Bottom,
    Stack,
}

#[inline]
pub fn preset_palette_ratio(preset_json: Option<&str>) -> Option<f32> {
    preset_json
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get("VoxelPaletteRatio").cloned())
        .and_then(|v| v.as_f64())
        .map(|r| (r as f32).clamp(0.02, 0.45))
}

pub fn apply_palette_ratio_once(st: &mut DockState<ViewportDockTab>, ratio: f32) {
    fn leaf_has_palette(tabs: &[ViewportDockTab]) -> bool {
        tabs.iter().any(|t| {
            matches!(
                t,
                ViewportDockTab::Coverlay(p) if matches!(p.kind, CoverlayPanelKind::VoxelPalette)
            )
        })
    }
    fn rec(tree: &mut Tree<ViewportDockTab>, idx: NodeIndex, ratio: f32) -> bool {
        let kind = {
            let t: &Tree<ViewportDockTab> = &*tree;
            match &t[idx] {
                Node::Leaf { tabs, .. } => return leaf_has_palette(tabs),
                Node::Empty => return false,
                Node::Horizontal { .. } => 1u8,
                Node::Vertical { .. } => 2u8,
                _ => 0u8,
            }
        };
        let left = idx.left();
        let right = idx.right();
        if kind == 1 {
            let l = rec(tree, left, ratio);
            let r = rec(tree, right, ratio);
            if l ^ r {
                if let Node::Horizontal { fraction, .. } = &mut tree[idx] {
                    *fraction = if l { ratio } else { 1.0 - ratio };
                }
            }
            return l || r;
        }
        if kind == 2 {
            let t = rec(tree, left, ratio);
            let b = rec(tree, right, ratio);
            if t ^ b {
                if let Node::Vertical { fraction, .. } = &mut tree[idx] {
                    *fraction = if t { ratio } else { 1.0 - ratio };
                }
            }
            return t || b;
        }
        false
    }
    let _ = rec(st.main_surface_mut(), NodeIndex::root(), ratio.clamp(0.02, 0.45));
}

pub fn coverlay_strip_runtime(mut d: DockState<ViewportDockTab>) -> DockState<ViewportDockTab> {
    for (_si, n) in d.iter_all_nodes_mut() {
        match n {
            Node::Leaf { rect, viewport, scroll, .. } => {
                *rect = bevy_egui::egui::Rect::NOTHING;
                *viewport = bevy_egui::egui::Rect::NOTHING;
                *scroll = 0.0;
            }
            Node::Vertical { rect, .. } | Node::Horizontal { rect, .. } => {
                *rect = bevy_egui::egui::Rect::NOTHING;
            }
            Node::Empty => {}
        }
    }
    d
}

/// Clamp split fractions so side/bottom panels can't steal the full viewport.
#[inline]
pub fn clamp_viewport_dock_fractions(
    st: &mut DockState<ViewportDockTab>,
    keep_x: f32,
    keep_y: f32,
) {
    let keep_x = keep_x.clamp(0.50, 0.98);
    let keep_y = keep_y.clamp(0.50, 0.98);
    fn leaf_has_viewport(tabs: &[ViewportDockTab]) -> bool {
        tabs.iter().any(|t| matches!(t, ViewportDockTab::Viewport))
    }
    fn rec(tree: &mut Tree<ViewportDockTab>, idx: NodeIndex, keep_x: f32, keep_y: f32) -> bool {
        let kind = {
            let t: &Tree<ViewportDockTab> = &*tree;
            match &t[idx] {
                Node::Leaf { tabs, .. } => return leaf_has_viewport(tabs),
                Node::Empty => return false,
                Node::Horizontal { .. } => 1u8,
                Node::Vertical { .. } => 2u8,
                _ => 0u8,
            }
        };
        let left = idx.left();
        let right = idx.right();
        if kind == 1 {
            let l = rec(tree, left, keep_x, keep_y);
            let r = rec(tree, right, keep_x, keep_y);
            if l ^ r {
                if let Node::Horizontal { fraction, .. } = &mut tree[idx] {
                    if l { *fraction = (*fraction).max(keep_x); } else { *fraction = (*fraction).min(1.0 - keep_x); }
                    *fraction = (*fraction).clamp(0.02, 0.98);
                }
            }
            return l || r;
        }
        if kind == 2 {
            let t = rec(tree, left, keep_x, keep_y);
            let b = rec(tree, right, keep_x, keep_y);
            if t ^ b {
                if let Node::Vertical { fraction, .. } = &mut tree[idx] {
                    if t { *fraction = (*fraction).max(keep_y); } else { *fraction = (*fraction).min(1.0 - keep_y); }
                    *fraction = (*fraction).clamp(0.02, 0.98);
                }
            }
            return t || b;
        }
        false
    }
    let tree = st.main_surface_mut();
    let _ = rec(tree, NodeIndex::root(), keep_x, keep_y);
}

#[inline]
pub fn coverlay_dock_sig(owner: Option<Uuid>, tabs: &[ViewportDockTab]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    let mut h = DefaultHasher::new();
    owner.hash(&mut h);
    tabs.len().hash(&mut h);
    for t in tabs {
        t.hash(&mut h);
    }
    h.finish()
}

pub fn build_default_viewport_dock_from_preset(
    preset_json: Option<String>,
    panels: Vec<CoverlayDockPanel>,
) -> DockState<ViewportDockTab> {
    let (preset, ratios): (HashMap<CoverlayPanelKind, DockSlot>, HashMap<CoverlayPanelKind, f32>) =
        preset_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.as_object().cloned())
            .map(|obj| {
                let mut slots: HashMap<CoverlayPanelKind, DockSlot> = HashMap::new();
                let mut ratios: HashMap<CoverlayPanelKind, f32> = HashMap::new();
                for (k, v) in obj {
                    if let Some(base) = k.strip_suffix("Ratio") {
                        let kind =
                            serde_json::from_str::<CoverlayPanelKind>(&format!("\"{}\"", base))
                                .ok();
                        let ratio = v.as_f64().map(|x| x as f32);
                        if let (Some(kind), Some(r)) = (kind, ratio) {
                            ratios.insert(kind, r.clamp(0.02, 0.30));
                        }
                        continue;
                    }
                    let kind =
                        serde_json::from_str::<CoverlayPanelKind>(&format!("\"{}\"", k)).ok();
                    let slot = serde_json::from_value::<DockSlot>(v).ok();
                    if let (Some(kind), Some(slot)) = (kind, slot) {
                        slots.insert(kind, slot);
                    }
                }
                (slots, ratios)
            })
            .unwrap_or_default();

    let by = |slot: DockSlot| -> Vec<CoverlayDockPanel> {
        panels
            .iter()
            .filter(|p| preset.get(&p.kind).copied().unwrap_or(DockSlot::Stack) == slot)
            .cloned()
            .collect()
    };

    let mut right = by(DockSlot::Right);
    let mut left = by(DockSlot::Left);
    let mut bottom = by(DockSlot::Bottom);
    let mut top = by(DockSlot::Top);
    let mut stack = by(DockSlot::Stack);

    if right.is_empty() && left.is_empty() && bottom.is_empty() && top.is_empty() && !stack.is_empty()
    {
        for p in stack.drain(..) {
            match p.kind {
                CoverlayPanelKind::VoxelTools
                | CoverlayPanelKind::SdfTools
                | CoverlayPanelKind::Manager
                | CoverlayPanelKind::Parameters => {
                    right.push(p)
                }
                CoverlayPanelKind::VoxelPalette => left.push(p),
                CoverlayPanelKind::VoxelDebug | CoverlayPanelKind::Anim => bottom.push(p),
                CoverlayPanelKind::Import | CoverlayPanelKind::Export => right.push(p),
                CoverlayPanelKind::NodeCoverlay => right.push(p),
            }
        }
    }

    let mut st = DockState::new(vec![ViewportDockTab::Viewport]);
    let mut vp_node = NodeIndex::root();
    let mut first_leaf: Option<NodeIndex> = None;

    fn place(
        st: &mut DockState<ViewportDockTab>,
        vp_node: &mut NodeIndex,
        tabs: &mut Vec<CoverlayDockPanel>,
        split: fn(&mut Tree<ViewportDockTab>, NodeIndex, f32, Vec<ViewportDockTab>) -> [NodeIndex; 2],
        keep: f32,
        invert_fraction: bool,
    ) -> Option<NodeIndex> {
        if tabs.is_empty() {
            return None;
        }
        let first = ViewportDockTab::Coverlay(tabs.remove(0));
        let keep = keep.clamp(0.50, 0.98);
        let fraction = if invert_fraction { 1.0 - keep } else { keep };
        let [old, new] = split(st.main_surface_mut(), *vp_node, fraction, vec![first]);
        *vp_node = old;
        for p in tabs.drain(..) {
            st.push_to_focused_leaf(ViewportDockTab::Coverlay(p));
        }
        Some(new)
    }

    let keep_left = ratios
        .get(&CoverlayPanelKind::VoxelPalette)
        .copied()
        .map(|r| 1.0 - r)
        .unwrap_or(VIEWPORT_KEEP_X);
    if let Some(n) = place(
        &mut st,
        &mut vp_node,
        &mut right,
        Tree::split_right,
        VIEWPORT_KEEP_X,
        false,
    ) {
        first_leaf.get_or_insert(n);
    }
    if let Some(n) = place(
        &mut st,
        &mut vp_node,
        &mut left,
        Tree::split_left,
        keep_left,
        true,
    ) {
        first_leaf.get_or_insert(n);
    }
    if let Some(n) = place(
        &mut st,
        &mut vp_node,
        &mut bottom,
        Tree::split_below,
        VIEWPORT_KEEP_Y,
        false,
    ) {
        first_leaf.get_or_insert(n);
    }
    if let Some(n) = place(
        &mut st,
        &mut vp_node,
        &mut top,
        Tree::split_above,
        VIEWPORT_KEEP_Y,
        true,
    ) {
        first_leaf.get_or_insert(n);
    }

    if !stack.is_empty() {
        if first_leaf.is_none() {
            let mut tmp = stack;
            let _ = place(&mut st, &mut vp_node, &mut tmp, Tree::split_right, VIEWPORT_KEEP_X, false);
        } else {
            for p in stack {
                st.push_to_focused_leaf(ViewportDockTab::Coverlay(p));
            }
        }
    }
    st
}

