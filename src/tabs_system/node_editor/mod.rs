#![allow(deprecated)]
use crate::tabs_system::{EditorTab, EditorTabContext};
use bevy_egui::egui::{self, Color32, Rect, Sense, Ui};
pub mod bars;
pub mod cda;
pub mod connection_hint;
pub mod drawing;
pub mod events;
pub mod gestures;
pub mod hud;
pub mod icons;
pub mod interactions;
pub mod mathematic;
pub mod menus;
pub mod node_info_tab;
pub mod state;

pub use state::NodeEditorTab;

use crate::cunning_core::command::basic::{
    CmdAddNetworkBox, CmdAddStickyNote, CmdBatch, CmdReplaceGraph, GraphSnapshot,
};
use crate::nodes::structs::NodeType;
use crate::tabs_system::rect_hash::mix_rect;
use bars::items::promote_note::{draw_promote_notes, poll_voice_events};
use bars::items::{draw_foreach_overlays, draw_network_boxes, draw_sticky_notes, handle_cut_tool};
use bars::toolbar::*;
use drawing::*;
use events::common::*;
use events::desktop::*;
use menus::context::*;
use menus::radial::*;
use state::NodeSnapshot;

#[inline]
fn hash_uuid_u64(id: uuid::Uuid) -> u64 {
    let v = id.as_u128();
    (v as u64) ^ ((v >> 64) as u64).rotate_left(17)
}

fn parse_first_json_value(raw: &str) -> Option<serde_json::Value> {
    use serde::Deserialize;
    let s = raw.trim();
    for (i, ch) in s.char_indices() {
        if ch != '{' && ch != '[' { continue; }
        let mut de = serde_json::Deserializer::from_str(&s[i..]);
        if let Ok(v) = serde_json::Value::deserialize(&mut de) { return Some(v); }
    }
    None
}

fn parse_nodes_fallback(raw: &str) -> Vec<String> {
    let s = raw.replace("```json", "```").replace("```", "\n");
    let mut out = Vec::new();
    for mut l in s.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(p) = l.find(':') { if l[..p].trim().eq_ignore_ascii_case("nodes") { l = &l[p + 1..]; } }
        l = l.trim_start_matches(|c: char| c == '-' || c == '*' || c == '•' || c.is_ascii_digit() || c == '.' || c == ')').trim();
        for part in l.split(',').map(str::trim) {
            let p = part.trim_matches(|c| c == '"' || c == '\'' || c == '[' || c == ']');
            if !p.is_empty() && !p.contains('{') && !p.contains('}') { out.push(p.to_string()); }
        }
        if out.len() >= 16 { break; }
    }
    out
}

fn wrap_text(s: &str, cols: usize) -> String {
    let cols = cols.max(24);
    let mut out = String::new();
    for (li, l0) in s.replace("\r\n", "\n").lines().enumerate() {
        if li > 0 { out.push('\n'); }
        let l = l0.trim_end();
        if l.is_empty() { continue; }
        let mut cur = String::new();
        let mut cur_len = 0usize;
        for w in l.split_whitespace() {
            let wl = w.chars().count();
            if cur_len > 0 && cur_len + 1 + wl > cols {
                out.push_str(cur.trim_end());
                out.push('\n');
                cur.clear();
                cur_len = 0;
            }
            if cur_len > 0 { cur.push(' '); cur_len += 1; }
            cur.push_str(w);
            cur_len += wl;
        }
        out.push_str(cur.trim_end());
    }
    out.trim().to_string()
}

fn auto_place_rect(g: &crate::nodes::structs::NodeGraph, desired: Rect, pad: f32) -> Rect {
    let blocked = |r: Rect| -> bool {
        for n in g.nodes.values() { if Rect::from_min_size(n.position, n.size).expand(pad).intersects(r) { return true; } }
        for s in g.sticky_notes.values() { if s.rect.expand(pad).intersects(r) { return true; } }
        for b in g.network_boxes.values() { if b.rect.expand(pad).intersects(r) { return true; } }
        false
    };
    if !blocked(desired) { return desired; }
    let step = 72.0;
    let c0 = desired.center();
    let sz = desired.size();
    for r in 1i32..=16 {
        for ox in -r..=r {
            for oy in -r..=r {
                if ox.abs() != r && oy.abs() != r { continue; }
                let c = c0 + egui::Vec2::new(ox as f32 * step, oy as f32 * step);
                let cand = Rect::from_center_size(c, sz);
                if !blocked(cand) { return cand; }
            }
        }
    }
    desired
}

fn layout_ai_module(
    g: &crate::nodes::structs::NodeGraph,
    settings: &crate::node_editor_settings::NodeEditorSettings,
    anchor: egui::Pos2,
    nodes: &mut [crate::nodes::Node],
    sticky: Option<&mut crate::nodes::StickyNote>,
) -> Rect {
    let sz = crate::node_editor_settings::resolved_node_size(settings);
    let gap_x = settings.ai_layout_gap_x.max(16.0);
    let gap_y = settings.ai_layout_gap_y.max(16.0);
    let max_cols = settings.ai_layout_max_cols.max(1) as usize;
    let cols = nodes.len().min(max_cols).max(1);
    let rows = (nodes.len() + cols - 1) / cols;
    let node_w = sz[0];
    let node_h = sz[1];
    let nodes_w = cols as f32 * node_w + (cols.saturating_sub(1) as f32) * gap_x;
    let nodes_h = rows as f32 * node_h + (rows.saturating_sub(1) as f32) * gap_y;
    let has_sticky = sticky.is_some();
    let (sticky_w, sticky_h) = sticky.as_ref().map(|n| (n.rect.width(), n.rect.height())).unwrap_or((0.0, 0.0));
    let sticky_gap = if has_sticky { 24.0 } else { 0.0 };
    let module_w = nodes_w.max(sticky_w);
    let module_h = (if has_sticky { sticky_h + sticky_gap } else { 0.0 }) + nodes_h;
    let desired = Rect::from_min_size(anchor, egui::Vec2::new(module_w, module_h))
        .expand(settings.ai_layout_box_pad.max(0.0));
    let placed = auto_place_rect(g, desired, settings.ai_layout_avoid_pad.max(0.0));
    let base = placed.min + egui::Vec2::splat(settings.ai_layout_box_pad.max(0.0));
    if let Some(n) = sticky {
        n.rect = Rect::from_min_size(base, egui::Vec2::new(sticky_w, sticky_h));
    }
    let y0 = base.y + (if has_sticky { sticky_h + sticky_gap } else { 0.0 });
    for (i, n) in nodes.iter_mut().enumerate() {
        let cx = (i % cols) as f32;
        let cy = (i / cols) as f32;
        n.position = egui::Pos2::new(base.x + cx * (node_w + gap_x), y0 + cy * (node_h + gap_y));
    }
    placed
}

/// Handle CDA operations
fn handle_cda_actions(editor: &mut NodeEditorTab, _ui: &mut Ui, context: &mut EditorTabContext) {
    if editor.create_cda_request {
        editor.create_cda_request = false;
        let selected: Vec<_> = context.ui_state.selected_nodes.iter().copied().collect();
        if !selected.is_empty() {
            editor.cda_state.open_create_dialog(selected);
        }
    }

    if let Some(ref info) = editor.cda_state.pending_create.take() {
        let root_graph = &mut context.node_graph_res.0;
        let path = editor.cda_state.breadcrumb();
        cda::navigation::with_graph_by_path_mut(root_graph, &path, |graph| {
            let before = GraphSnapshot::capture(graph);
            let result = crate::nodes::cda::cda_node::create_cda_from_nodes(
                &info.name,
                graph,
                &info.selected_nodes,
                context.node_editor_settings,
            );
            let cda_asset = result.asset;

            // Compute CDA node position (center of selected nodes)
            let positions: Vec<_> = info
                .selected_nodes
                .iter()
                .filter_map(|id| graph.nodes.get(id).map(|n| n.position))
                .collect();
            let center = if positions.is_empty() {
                egui::Pos2::new(100.0, 100.0)
            } else {
                let sum = positions
                    .iter()
                    .fold(egui::Vec2::ZERO, |acc, p| acc + p.to_vec2());
                (sum / positions.len() as f32).to_pos2()
            };

            let cda_node_id = uuid::Uuid::new_v4();
            let selected_set: std::collections::HashSet<_> =
                info.selected_nodes.iter().copied().collect();

            // Rewiring Phase: 1. Input rewiring
            for ((ext_node, ext_port), cda_input_port) in result.external_inputs {
                let conn_id = uuid::Uuid::new_v4();
                let new_conn = crate::nodes::structs::Connection {
                    id: conn_id,
                    from_node: ext_node,
                    from_port: ext_port,
                    to_node: cda_node_id,
                    to_port: cda_input_port,
                    order: 0,
                    waypoints: Vec::new(),
                };
                graph.connections.insert(conn_id, new_conn);
            }

            // 2. Output rewiring: find all outgoing edges from (int_node, int_port)
            let mut outgoing_index: std::collections::HashMap<
                (uuid::Uuid, crate::nodes::PortId),
                Vec<crate::nodes::structs::Connection>,
            > = std::collections::HashMap::new();
            for conn in graph.connections.values() {
                if selected_set.contains(&conn.from_node) {
                    outgoing_index
                        .entry((conn.from_node, conn.from_port))
                        .or_default()
                        .push(conn.clone());
                }
            }

            for ((int_node, int_port), cda_output_port) in result.external_outputs {
                if let Some(edges) = outgoing_index.get(&(int_node, int_port)) {
                    for edge in edges {
                        // Only rewire edges pointing to external nodes
                        if !selected_set.contains(&edge.to_node) {
                            let conn_id = uuid::Uuid::new_v4();
                            let new_conn = crate::nodes::structs::Connection {
                                id: conn_id,
                                from_node: cda_node_id,
                                from_port: cda_output_port.clone(),
                                to_node: edge.to_node,
                                to_port: edge.to_port.clone(),
                                order: 0,
                                waypoints: Vec::new(),
                            };
                            graph.connections.insert(conn_id, new_conn);
                        }
                    }
                }
            }

            // Cleanup Phase: remove selected nodes
            for node_id in &info.selected_nodes {
                graph.nodes.remove(node_id);
            }
            graph.connections.retain(|_, c| {
                !selected_set.contains(&c.from_node) && !selected_set.contains(&c.to_node)
            });

            // Insertion Phase: insert CDA node (instance holds asset_ref only; definition is stored in CdaLibrary)
            let asset_ref = crate::cunning_core::cda::library::global_cda_library()
                .map(|lib| lib.insert_in_memory(cda_asset))
                .unwrap_or(crate::cunning_core::cda::CdaAssetRef {
                    uuid: uuid::Uuid::new_v4(),
                    path: String::new(),
                });
            let cda_data = crate::nodes::structs::CDANodeData {
                asset_ref: asset_ref.clone(),
                name: info.name.clone(),
                coverlay_hud: None,
                coverlay_units: Vec::new(),
                inner_param_overrides: Default::default(),
            };
            let mut cda_node = crate::nodes::structs::Node::new(
                cda_node_id,
                info.name.clone(),
                NodeType::CDA(cda_data),
                center,
            );
            let default_size =
                crate::node_editor_settings::resolved_node_size(context.node_editor_settings);
            cda_node.size = bevy_egui::egui::vec2(default_size[0], default_size[1]);
            graph.nodes.insert(cda_node_id, cda_node);

            context.ui_state.selected_nodes.clear();
            context.ui_state.selected_nodes.insert(cda_node_id);
            let after = GraphSnapshot::capture(graph);
            context
                .node_editor_state
                .record(Box::new(CmdReplaceGraph::new(before, after)));
            context.graph_changed_writer.write_default();
        });
    }
}

impl EditorTab for NodeEditorTab {
    fn ui(&mut self, ui: &mut Ui, context: &mut EditorTabContext) {
        ui.push_id(self.tab_id, |ui| {
            // Sync state
            self.pan = context.node_editor_state.pan;
            self.zoom = context.node_editor_state.zoom;
            self.target_pan = context.node_editor_state.target_pan;
            self.target_zoom = context.node_editor_state.target_zoom;

            // Clear transient state (snap lines) at the start of the frame
            self.snap_lines.clear();

            // Fix for 3D Viewport bleed-through
            ui.painter().rect_filled(ui.clip_rect(), 0.0, ui.visuals().panel_fill);

            let full_rect = ui.available_rect_before_wrap();
            let top_bar_height = 32.0;
            
            // --- 1. Draw Top Bar ---
            let top_bar_rect = Rect::from_min_size(full_rect.min, bevy_egui::egui::vec2(full_rect.width(), top_bar_height));
            ui.allocate_ui_at_rect(top_bar_rect, |ui| { draw_top_bar(self, ui, context); });

            // --- 2. Setup Main Layout (Sidebar + Canvas) ---
            let main_rect = Rect::from_min_max(
                bevy_egui::egui::pos2(full_rect.min.x, full_rect.min.y + top_bar_height),
                full_rect.max
            );
            
            let sidebar_width = 32.0;

            let sidebar_rect = Rect::from_min_size(main_rect.min, bevy_egui::egui::vec2(sidebar_width, main_rect.height()));
            let canvas_rect = Rect::from_min_max(
                bevy_egui::egui::pos2(main_rect.min.x + sidebar_width, main_rect.min.y),
                main_rect.max
            );

            // Draw Sidebar
            ui.allocate_ui_at_rect(sidebar_rect, |ui| {
                draw_sidebar(self, ui);
            });

            // Canvas Area - Allocate interaction rect
            let response = ui.allocate_rect(canvas_rect, Sense::click_and_drag());
            
            // Reset single-frame state
            self.did_start_connection_this_frame = false;
            self.shaken_node_id = None;

            // Use canvas_rect for all calculations
            let editor_rect = canvas_rect;
            let editor_rect_id = mix_rect(0, editor_rect);

            // --- Event Handling ---
            handle_pan_zoom(self, ui, context, &response, editor_rect);
            // Keep local view state in sync within the same frame (wheel/pinch can update pan/zoom here).
            self.pan = context.node_editor_state.pan;
            self.zoom = context.node_editor_state.zoom;
            self.target_pan = context.node_editor_state.target_pan;
            self.target_zoom = context.node_editor_state.target_zoom;
            handle_shortcuts(self, ui, context, editor_rect);

            // Ensure cache is available for hit-testing (hover/box-select) without locking per node.
            // Determine whether to show main graph or CDA inner graph based on cda_state
            let cda_depth = self.cda_state.depth();
            if cda_depth > 0 {
                let root_graph = &mut context.node_graph_res.0;
                if cda::navigation::sync_current_cda_input_titles(root_graph, &self.cda_state) {
                    self.cached_nodes_rev = 0;
                    self.cached_topo_key = 0;
                    context.graph_changed_writer.write_default();
                }
            }
            let topo_key = {
                let root_graph = &context.node_graph_res.0;
                let display_graph = cda::navigation::graph_snapshot_by_path(root_graph, &self.cda_state.breadcrumb());
                // NOTE: `context.graph_revision` is a coarse UI invalidation counter (includes parameter drags).
                // For node editor layout caches, we want a key that ignores parameter churn and only changes when
                // the graph topology/structure changes (nodes/edges/display node).
                (display_graph.nodes.len() as u64)
                    ^ ((display_graph.connections.len() as u64).rotate_left(11))
                    ^ (display_graph.display_node.map(hash_uuid_u64).unwrap_or(0).rotate_left(23))
                    ^ ((cda_depth as u64).rotate_left(3))
                    ^ display_graph.graph_revision.rotate_left(7)
            };
            let need_rebuild_cache = self.cached_nodes.is_empty() || self.cached_topo_key != topo_key || self.cached_cda_depth != cda_depth;
            if need_rebuild_cache {
                let root_graph = &context.node_graph_res.0;
                let display_graph = cda::navigation::graph_snapshot_by_path(root_graph, &self.cda_state.breadcrumb());
                self.cached_nodes.clear();
                let display_id = display_graph.display_node;
                for n in display_graph.nodes.values() {
                    let inputs = { let mut v: Vec<_> = n.inputs.keys().cloned().collect(); v.sort_by(|a,b| crate::nodes::port_key::port_sort_key(a).cmp(&crate::nodes::port_key::port_sort_key(b)).then_with(|| a.as_str().cmp(b.as_str()))); v };
                    let outputs= { let mut v: Vec<_> = n.outputs.keys().cloned().collect(); v.sort_by(|a,b| crate::nodes::port_key::port_sort_key(a).cmp(&crate::nodes::port_key::port_sort_key(b)).then_with(|| a.as_str().cmp(b.as_str()))); v };
                    let (size, header_h) = drawing::compute_auto_node_layout(ui.ctx(), context.node_editor_settings, context.node_registry, &n.name, &inputs, &outputs, n.input_style);
                    self.cached_nodes.push(NodeSnapshot {
                        id: n.id, name: n.name.clone(), position: n.position, size, header_h, input_style: n.input_style, style: n.style,
                        is_template: n.is_template, is_bypassed: n.is_bypassed, is_display_node: display_id == Some(n.id), is_locked: n.is_locked,
                        inputs, outputs,
                    });
                }
                self.cached_nodes_rev = context.graph_revision;
                self.cached_topo_key = topo_key;
                self.cached_cda_depth = cda_depth;
            } else {
                // Fast refresh: update only cheap per-node fields that can change frequently (position, name, flags),
                // without recomputing layout/ports for every GraphChanged tick (e.g. gizmo param drags).
                //
                // CRITICAL: do NOT overwrite cached positions while dragging nodes.
                // Node dragging updates both the graph and `cached_nodes` during the gesture; clobbering here
                // causes "jitter while dragging, snap on release".
                let is_dragging_nodes = context.ui_state.dragged_node_id.is_some() || !self.drag_start_positions.is_empty();
                let root_graph = &context.node_graph_res.0;
                let display_graph = cda::navigation::graph_snapshot_by_path(root_graph, &self.cda_state.breadcrumb());
                let display_id = display_graph.display_node;
                for sn in self.cached_nodes.iter_mut() {
                    if let Some(n) = display_graph.nodes.get(&sn.id) {
                        sn.name = n.name.clone();
                        if !is_dragging_nodes { sn.position = n.position; }
                        sn.input_style = n.input_style;
                        sn.style = n.style;
                        sn.is_template = n.is_template;
                        sn.is_bypassed = n.is_bypassed;
                        sn.is_display_node = display_id == Some(n.id);
                        sn.is_locked = n.is_locked;
                    } else {
                        // Node set changed unexpectedly; force rebuild next frame.
                        self.cached_topo_key = 0;
                    }
                }
            }

            update_node_animations(self, ui, context);
            handle_deferred_actions(self, context, editor_rect);

            // Refresh cached node geometry after any mutations above (graph edits / animations / drags).
            if self.cached_nodes_rev != context.graph_revision || !self.node_animations.is_empty() || self.cached_cda_depth != cda_depth {
                let root_graph = &context.node_graph_res.0;
                let display_graph = cda::navigation::graph_snapshot_by_path(
                    root_graph,
                    &self.cda_state.breadcrumb(),
                );
                self.cached_nodes.clear();
                    let display_id = display_graph.display_node;
                    for n in display_graph.nodes.values() {
                        let inputs = { let mut v: Vec<_> = n.inputs.keys().cloned().collect(); v.sort_by(|a,b| crate::nodes::port_key::port_sort_key(a).cmp(&crate::nodes::port_key::port_sort_key(b)).then_with(|| a.as_str().cmp(b.as_str()))); v };
                        let outputs= { let mut v: Vec<_> = n.outputs.keys().cloned().collect(); v.sort_by(|a,b| crate::nodes::port_key::port_sort_key(a).cmp(&crate::nodes::port_key::port_sort_key(b)).then_with(|| a.as_str().cmp(b.as_str()))); v };
                        let (size, header_h) = drawing::compute_auto_node_layout(ui.ctx(), context.node_editor_settings, context.node_registry, &n.name, &inputs, &outputs, n.input_style);
                        self.cached_nodes.push(NodeSnapshot {
                            id: n.id, name: n.name.clone(), position: n.position, size, header_h, input_style: n.input_style, style: n.style,
                            is_template: n.is_template, is_bypassed: n.is_bypassed, is_display_node: display_id == Some(n.id), is_locked: n.is_locked,
                            inputs, outputs,
                        });
                    }
                    self.cached_nodes_rev = context.graph_revision;
                    self.cached_cda_depth = cda_depth;
            }

            // Port locations are screen-space (depend on zoom/pan): rebuild every frame.
            drawing::rebuild_port_locations(self, context.node_editor_settings, editor_rect);

            // Hit cache is graph-space (zoom/pan invariant): rebuild only when graph/geometry changes.
            let hit_key = self.cached_topo_key ^ self.geometry_rev.rotate_left(1) ^ (self.cached_cda_depth as u64);
            drawing::rebuild_hit_cache(self, context.node_editor_settings, editor_rect, hit_key);

            // --- Box Selection Logic (event-driven; O(1) bucket lookup) ---
            let mut is_hovering_node = false;
            if let Some(p) = ui.ctx().pointer_interact_pos() {
                if editor_rect.contains(p) {
                    let gp = ((p - editor_rect.min - self.pan) / self.zoom).to_pos2();
                    let bs = self.hit_cache.bucket_size;
                    let key = ((gp.x / bs).floor() as i32, (gp.y / bs).floor() as i32);
                    if let Some(v) = self.hit_cache.buckets.get(&key) {
                        for &i in v.iter().rev() {
                            if let Some(n) = self.hit_cache.nodes.get(i) {
                                // Only treat pointer as "hovering node" when over the actual body rect.
                                // Using expanded visual rect here can disable box selection near nodes.
                                if n.logical_rect.contains(gp) { is_hovering_node = true; break; }
                            }
                        }
                    }
                }
            }
            let is_multi_touching = ui.input(|i| i.multi_touch().is_some());
            // Allow marquee selection to continue even if the pointer moves over a node mid-drag,
            // otherwise `selection_start` can get "stuck" because we stop calling `handle_box_selection`
            // and never observe the drag-stop event.
            if !self.is_cutting
                && !is_multi_touching
                && (!is_hovering_node || self.selection_start.is_some())
            {
                handle_box_selection(self, ui, &response, context, editor_rect);
            }
            if is_multi_touching { self.selection_start = None; }

            // --- Drawing ---
            // Deep mode ripple: start when deep_mode becomes active with a request
            let time = ui.input(|i| i.time) as f32;
            if self.deep_mode && self.ghost_request_id.is_some() && self.grid_ripple_start.is_none() {
                self.grid_ripple_start = Some(time);
            }
            if !self.deep_mode || self.ghost_request_id.is_none() {
                self.grid_ripple_start = None;
            }
            let ripple = self.grid_ripple_start.map(|start| {
                // Ripple center: wire endpoint or screen center
                let center = ui.ctx().pointer_interact_pos().unwrap_or(editor_rect.center());
                drawing::GridRippleState { active: true, center, start_time: start }
            });
            ui.allocate_ui_at_rect(editor_rect, |ui| {
                ui.set_clip_rect(editor_rect);
                ui.push_id("ne_grid", |ui| {
                    drawing::draw_grid(ui, editor_rect, self.pan, self.zoom, context.node_editor_settings, context.ui_invalidator, ripple.as_ref());
                });
            });
            ui.allocate_ui_at_rect(editor_rect, |ui| {
                ui.set_clip_rect(editor_rect);
                drawing::handle_node_input(self, ui, context, editor_rect);
            });

            ui.allocate_ui_at_rect(editor_rect, |ui| {
                ui.set_clip_rect(editor_rect);
                draw_foreach_overlays(ui, self, context, editor_rect);
                draw_network_boxes(ui, self, context, editor_rect);
                draw_sticky_notes(ui, self, context, editor_rect);
                draw_promote_notes(ui, self, context, editor_rect);
            });
            poll_voice_events(self, context);

            ui.allocate_ui_at_rect(editor_rect, |ui| {
                ui.set_clip_rect(editor_rect);
                ui.push_id(("ne_nodes", editor_rect_id), |ui| {
                    drawing::paint_nodes_retained(
                        ui,
                        self,
                        context,
                        editor_rect,
                        &self.cached_nodes,
                        1.0,
                        Some(&context.ui_state.selected_nodes),
                    );
                });
            });

            // Alignment snap-lines (same visual language as other dashed guides)
            ui.allocate_ui_at_rect(editor_rect, |ui| {
                ui.set_clip_rect(editor_rect);
                if !self.snap_lines.is_empty() {
                    let snap_color = Color32::from_gray(180);
                    for (p1, p2) in &self.snap_lines {
                        drawing::draw_dashed_line(ui, [*p1, *p2], snap_color, 5.0, 5.0);
                    }
                }
            });

            handle_radial_menu(self, ui, context, editor_rect);
            // Connections + previews must be clipped to the node-editor rect (prevents bleed into 3D viewport).
            ui.allocate_ui_at_rect(editor_rect, |ui| {
                ui.set_clip_rect(editor_rect);
                ui.push_id(("ne_links", editor_rect_id), |ui| {
                    let root_graph = &context.node_graph_res.0;
                    cda::navigation::with_graph_by_path(root_graph, &self.cda_state.breadcrumb(), |g| {
                        draw_connections(ui, g, context, &self.port_locations, editor_rect, self.pan, self.zoom, 1.0);
                    });
                });
                // Connection/port interaction feedback (hover + click-to-start).
                drawing::draw_port_feedback(self, ui, context.node_editor_settings, context.ui_invalidator);
                draw_insertion_preview(self, ui, context, editor_rect);
                draw_connection_preview(self, ui, context, editor_rect);
                drawing::draw_copilot_relays(self, ui, context, editor_rect);
            });

            // Poll Copilot backend for ghost completion results (single-wire + multi-wire aggregation).
            let inflight = self.copilot_inflight_backend.unwrap_or(self.copilot_backend);
            // Timeout detection for auto-retry
            use crate::libs::ai_service::ai_defaults::{GHOST_MAX_RETRIES, GHOST_TIMEOUT_SECS};
            if let (Some(_), Some(start)) = (&self.ghost_request_id, self.copilot_request_start) {
                if start.elapsed().as_secs_f32() > GHOST_TIMEOUT_SECS && !self.deep_mode {
                    if inflight == crate::tabs_system::node_editor::state::CopilotBackend::LocalTiny && self.copilot_retry_count < GHOST_MAX_RETRIES {
                        bevy::log::warn!("GhostCopilot: LocalTiny timeout ({:.1}s), retrying with LocalThink", start.elapsed().as_secs_f32());
                        self.ghost_request_id = None;
                        self.copilot_inflight_backend = None;
                        self.copilot_request_start = None;
                        self.copilot_retry_count += 1;
                        self.copilot_backend = crate::tabs_system::node_editor::state::CopilotBackend::LocalThink;
                        crate::tabs_system::node_editor::events::common::request_ghost_path(self, ui, context, editor_rect);
                    } else {
                        bevy::log::warn!("GhostCopilot: timeout after retries, giving up");
                        self.ghost_request_id = None;
                        self.copilot_inflight_backend = None;
                        self.copilot_request_start = None;
                    }
                }
            }
            if let Some(req_id) = self.ghost_request_id.clone() {
                let mut got: Option<(String, Option<String>)> = None; // (text, error)
                // Deep mode uses NativeAiHost (4B Thinking), poll from ai_events
                if self.deep_mode {
                    use crate::libs::ai_service::native_candle::AiResultEvent;
                    let mut full_text = String::new();
                    let mut done = false;
                    let mut err_msg: Option<String> = None;
                    for ev in context.ai_events.iter() {
                        match ev {
                            AiResultEvent::StreamChunk { id, text, done: d } if id == &req_id => { full_text.push_str(text); if *d { done = true; } }
                            AiResultEvent::Success { id, result } if id == &req_id => { full_text = result.clone(); done = true; }
                            AiResultEvent::Error { id, message } if id == &req_id => { err_msg = Some(message.clone()); done = true; }
                            _ => {}
                        }
                    }
                    if done { got = Some((full_text, err_msg)); }
                } else {
                    match inflight {
                        crate::tabs_system::node_editor::state::CopilotBackend::LocalTiny => {
                            if let Some(host) = context.tiny_model_host { for r in host.poll() { if r.id == req_id { got = Some((r.text, r.error)); break; } } }
                            else { bevy::log::warn!("GhostCopilot: LocalTiny inflight but tiny_model_host is None"); }
                        }
                        crate::tabs_system::node_editor::state::CopilotBackend::LocalThink => {
                            // LocalThink uses NativeAiHost (4B), poll from ai_events
                            use crate::libs::ai_service::native_candle::AiResultEvent;
                            let mut full_text = String::new();
                            let mut done = false;
                            let mut err_msg: Option<String> = None;
                            for ev in context.ai_events.iter() {
                                match ev {
                                    AiResultEvent::StreamChunk { id, text, done: d } if id == &req_id => { full_text.push_str(text); if *d { done = true; } }
                                    AiResultEvent::Success { id, result } if id == &req_id => { full_text = result.clone(); done = true; }
                                    AiResultEvent::Error { id, message } if id == &req_id => { err_msg = Some(message.clone()); done = true; }
                                    _ => {}
                                }
                            }
                            if done { got = Some((full_text, err_msg)); }
                        }
                        crate::tabs_system::node_editor::state::CopilotBackend::Gemini => {
                            if let Some(host) = context.gemini_copilot_host { for r in host.poll() { if r.id == req_id { got = Some((r.text, r.error)); break; } } }
                            else { bevy::log::warn!("GhostCopilot: Gemini inflight but gemini_copilot_host is None"); }
                        }
                    }
                }
                if let Some((text, err)) = got {
                    self.ghost_request_id = None;
                    self.copilot_inflight_backend = None;
                    let anchor = self.ghost_anchor_graph_pos.unwrap_or_else(|| egui::Pos2::new(0.0, 0.0));
                    if text.trim().is_empty() {
                        bevy::log::warn!("GhostCopilot: empty response (backend={:?}) err={:?}", inflight, err);
                        // Auto-fallback: Gemini quota/rate limit -> disable cloud briefly and retry locally
                        if !self.deep_mode
                            && inflight == crate::tabs_system::node_editor::state::CopilotBackend::Gemini
                            && err.as_deref().is_some_and(|e| e.contains("HTTP 429") || e.contains("RESOURCE_EXHAUSTED"))
                        {
                            fn retry_after_secs(e: &str) -> Option<u64> {
                                let k = "\"retryDelay\": \"";
                                let i = e.find(k)? + k.len();
                                let s = &e[i..];
                                let j = s.find('s')?;
                                s[..j].trim().parse::<u64>().ok()
                            }
                            let secs = err.as_deref().and_then(retry_after_secs).unwrap_or(30);
                            self.copilot_cloud_disabled_until =
                                Some(std::time::Instant::now() + std::time::Duration::from_secs(secs));
                            self.ghost_reason_title = Some("Gemini unavailable".to_string());
                            self.ghost_reason = Some(format!("Cloud quota/rate limit. Falling back to local for {}s.", secs));
                            self.copilot_retry_count = 0;
                            crate::tabs_system::node_editor::events::common::request_ghost_path(self, ui, context, editor_rect);
                            return;
                        }
                        // Auto-retry: switch to LocalThink if LocalTiny failed
                        if !self.deep_mode && inflight == crate::tabs_system::node_editor::state::CopilotBackend::LocalTiny && self.copilot_retry_count < GHOST_MAX_RETRIES {
                            bevy::log::info!("GhostCopilot: retrying with LocalThink...");
                            self.copilot_retry_count += 1;
                            self.copilot_backend = crate::tabs_system::node_editor::state::CopilotBackend::LocalThink;
                            crate::tabs_system::node_editor::events::common::request_ghost_path(self, ui, context, editor_rect);
                        }
                        return;
                    }

                    // Deep mode: check for skill_call and execute multi-turn loop
                    if self.deep_mode {
                        use crate::libs::ai_service::copilot_skill::{try_parse_skill_call, execute_skill, SkillResult, MAX_SKILL_TURNS};
                        // Strip thinking tags if present
                        let clean_text = {
                            let mut t = text.clone();
                            if let Some(start) = t.find("<think>") {
                                if let Some(end) = t.find("</think>") { t = format!("{}{}", &t[..start], &t[end + 8..]); }
                            }
                            t.trim().to_string()
                        };
                        if let Some(call) = try_parse_skill_call(&clean_text) {
                            if self.deep_skill_turns < MAX_SKILL_TURNS {
                                bevy::log::info!("DeepCopilot: skill_call {:?}", call);
                                let result = execute_skill(&call);
                                let result_txt = match result {
                                    SkillResult::Text(s) => s,
                                    SkillResult::Json(v) => serde_json::to_string_pretty(&v).unwrap_or_default(),
                                };
                                // Append to deep_history
                                self.deep_history.push_str(&format!("<|im_start|>assistant\n{}<|im_end|>\n<|im_start|>system\n[Skill Result]\n{}<|im_end|>\n", clean_text, result_txt));
                                self.deep_skill_turns += 1;
                                // Continue: re-request
                                crate::tabs_system::node_editor::events::common::request_ghost_path(self, ui, context, editor_rect);
                                return;
                            } else {
                                bevy::log::warn!("DeepCopilot: max skill turns reached");
                            }
                        }
                        // Not a skill call or max turns reached: parse as final output
                    }

                    fn parse_ghost(raw: &str) -> (Vec<String>, Option<String>, Option<String>, std::collections::HashMap<String, String>) {
                        use serde_json::Value;
                        let Some(v) = parse_first_json_value(raw) else { return (parse_nodes_fallback(raw), None, None, std::collections::HashMap::new()); };
                        if let Value::Array(a) = v {
                            let nodes = a.into_iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect::<Vec<_>>();
                            return (nodes, None, None, std::collections::HashMap::new());
                        }
                        let Value::Object(o) = v else { return (Vec::new(), None, None, std::collections::HashMap::new()); };
                        let nodes = o.get("nodes").and_then(|n| n.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect::<Vec<_>>()).unwrap_or_default();
                        let rt = o.get("reason_title").and_then(|x| x.as_str()).map(|s| s.to_string());
                        let r = o.get("reason").and_then(|x| x.as_str()).map(|s| s.to_string());
                        let params = o.get("params").and_then(|p| p.as_object()).map(|m| {
                            m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))
                                .or_else(|| v.as_bool().map(|b| (k.clone(), b.to_string())))
                                .or_else(|| v.as_i64().map(|i| (k.clone(), i.to_string())))
                            ).collect()
                        }).unwrap_or_default();
                        (nodes, rt, r, params)
                    }
                    let (mut names, reason_title, reason, params) = parse_ghost(&text);
                    if names.is_empty() {
                        let preview: String = text.chars().take(240).collect();
                        bevy::log::warn!("GhostCopilot: parse fail (backend={:?}) preview={:?}", inflight, preview);
                        // Auto-retry: switch to LocalThink if LocalTiny failed
                        if !self.deep_mode && inflight == crate::tabs_system::node_editor::state::CopilotBackend::LocalTiny && self.copilot_retry_count < GHOST_MAX_RETRIES {
                            bevy::log::info!("GhostCopilot: retrying with LocalThink...");
                            self.copilot_retry_count += 1;
                            self.copilot_backend = crate::tabs_system::node_editor::state::CopilotBackend::LocalThink;
                            crate::tabs_system::node_editor::events::common::request_ghost_path(self, ui, context, editor_rect);
                        }
                        return;
                    }
                    self.ghost_reason_title = reason_title;
                    self.ghost_reason = reason;
                    self.ghost_params = params;
                    self.copilot_retry_count = 0;
                    self.copilot_request_start = None;
                    if let Some(u) = self.ghost_pending_user.take() {
                        self.ghost_dialog.push_str("<|im_start|>user\n");
                        self.ghost_dialog.push_str(&u);
                        self.ghost_dialog.push_str("\n<|im_end|>\n<|im_start|>assistant\n");
                        self.ghost_dialog.push_str(text.trim());
                        self.ghost_dialog.push_str("\n<|im_end|>\n");
                        self.ghost_turns = self.ghost_turns.saturating_add(1);
                    }
                    let sz = crate::node_editor_settings::resolved_node_size(context.node_editor_settings);
                    let mut nodes = Vec::new();
                    let mut links = Vec::new();
                    let stride = (sz[0] + 80.0).max(220.0);
                    let mut prev: Option<crate::nodes::NodeId> = None;
                    let knowledge = crate::libs::ai_service::native_tiny_model::knowledge_cache::GLOBAL_KNOWLEDGE_CACHE
                        .get()
                        .map(|k| k.nodes.clone());

                    // Multi-wire: first response can be an aggregator selection.
                    let mut handled = false;
                    if req_id.starts_with("ghost_agg_") {
                        let agg = names.remove(0);
                        let agg_name = if agg.is_empty() { "Merge".to_string() } else { agg };
                        self.ghost_multi_agg_name = Some(agg_name.clone());
                        self.ghost_reason_title = None;
                        self.ghost_reason = None;

                        let id = crate::nodes::NodeId::new_v4();
                        let pos = egui::pos2(anchor.x, anchor.y);
                        let (inputs, input_style) = if let Some(map) = knowledge.as_ref() {
                            if let Some(k) = map.get(&agg_name) {
                                let ins: Vec<crate::nodes::PortId> = k.io.inputs.iter().map(|p| crate::nodes::PortId::from(p.name.as_str())).collect();
                                let style = if ins.len() > 1 || k.io.input_type == "Multi" { crate::nodes::InputStyle::Collection } else { crate::nodes::InputStyle::Individual };
                                (ins, style)
                            } else { (Vec::new(), crate::nodes::InputStyle::Individual) }
                        } else { (Vec::new(), crate::nodes::InputStyle::Individual) };
                        let outputs = if let Some(map) = knowledge.as_ref() {
                            map.get(&agg_name).map(|k| k.io.outputs.iter().map(|p| crate::nodes::PortId::from(p.name.as_str())).collect()).unwrap_or_else(|| vec![crate::nodes::PortId::from("Output")])
                        } else { vec![crate::nodes::PortId::from("Output")] };
                        let mut agg_nodes = Vec::new();
                        agg_nodes.push(crate::tabs_system::node_editor::state::NodeSnapshot {
                            id,
                            name: agg_name.clone(),
                            position: pos,
                            size: egui::vec2(sz[0], sz[1]),
                            header_h: context.node_editor_settings.node_header_h_base.max(10.0),
                            input_style,
                            style: crate::nodes::NodeStyle::Normal,
                            is_template: false,
                            is_bypassed: false,
                            is_display_node: false,
                            is_locked: false,
                            inputs,
                            outputs,
                        });
                        self.ghost_graph = Some(crate::tabs_system::node_editor::state::GhostGraph { nodes: agg_nodes, links: Vec::new() });
                        self.ghost_tab_last_time = None;
                        crate::tabs_system::node_editor::events::common::request_ghost_path_or_multi(self, ui, context, editor_rect);
                        handled = true;
                    }

                    if !handled {
                        // Multi-wire path completion: if aggregator already exists, append downstream nodes.
                        if req_id.starts_with("ghost_pathmw_") {
                            if let Some(g) = self.ghost_graph.as_ref() {
                                if let Some(first) = g.nodes.first() { prev = Some(first.id); }
                            }
                        }
                        for (i, name) in names.into_iter().enumerate() {
                            let id = crate::nodes::NodeId::new_v4();
                            let base_i = if prev.is_some() { i + 1 } else { i };
                            let pos = egui::pos2(anchor.x + base_i as f32 * stride, anchor.y);
                            let (inputs, input_style) = if let Some(map) = knowledge.as_ref() {
                                if let Some(k) = map.get(&name) {
                                    let ins: Vec<crate::nodes::PortId> = k.io.inputs.iter().map(|p| crate::nodes::PortId::from(p.name.as_str())).collect();
                                    let style = if ins.len() > 1 || k.io.input_type == "Multi" { crate::nodes::InputStyle::Collection } else { crate::nodes::InputStyle::Individual };
                                    (ins, style)
                                } else { (Vec::new(), crate::nodes::InputStyle::Individual) }
                            } else { (Vec::new(), crate::nodes::InputStyle::Individual) };
                            let outputs = if let Some(map) = knowledge.as_ref() {
                                map.get(&name).map(|k| k.io.outputs.iter().map(|p| crate::nodes::PortId::from(p.name.as_str())).collect()).unwrap_or_else(|| vec![crate::nodes::PortId::from("Output")])
                            } else { vec![crate::nodes::PortId::from("Output")] };
                            nodes.push(crate::tabs_system::node_editor::state::NodeSnapshot {
                                id,
                                name,
                                position: pos,
                                size: egui::vec2(sz[0], sz[1]),
                                header_h: context.node_editor_settings.node_header_h_base.max(10.0),
                                input_style,
                                style: crate::nodes::NodeStyle::Normal,
                                is_template: false,
                                is_bypassed: false,
                                is_display_node: false,
                                is_locked: false,
                                inputs,
                                outputs,
                            });
                            if let Some(p) = prev { links.push((p, id)); }
                            prev = Some(id);
                        }
                        if req_id.starts_with("ghost_pathmw_") {
                            if let Some(g) = self.ghost_graph.as_mut() {
                                if !nodes.is_empty() { g.nodes.extend(nodes); g.links.extend(links); }
                            } else {
                                self.ghost_graph = Some(crate::tabs_system::node_editor::state::GhostGraph { nodes, links });
                            }
                        } else {
                            self.ghost_graph = Some(crate::tabs_system::node_editor::state::GhostGraph { nodes, links });
                        }
                        self.ghost_tab_last_time = None;
                    }
                }
            }

            // Poll Copilot relay sessions (UI-only, concurrent).
            if !self.copilot_relays.is_empty() {
                let relay_ids: Vec<uuid::Uuid> = self.copilot_relays.keys().copied().collect();
                for sid in relay_ids {
                    let (backend, req_id, anchor, target_nodes, status) = if let Some(s) = self.copilot_relays.get(&sid) {
                        (s.backend, s.request_id.clone(), s.anchor_graph_pos, s.target_nodes, s.status)
                    } else {
                        continue;
                    };
                    if status != crate::tabs_system::node_editor::state::CopilotRelayStatus::Generating {
                        continue;
                    }
                    let mut got: Option<(String, Option<String>)> = None;
                    match backend {
                        crate::tabs_system::node_editor::state::CopilotBackend::LocalTiny => {
                            if let Some(host) = context.tiny_model_host {
                                for r in host.poll() {
                                    if r.id == req_id {
                                        got = Some((r.text, r.error));
                                        break;
                                    }
                                }
                            }
                        }
                        crate::tabs_system::node_editor::state::CopilotBackend::LocalThink => {
                            use crate::libs::ai_service::native_candle::AiResultEvent;
                            let mut full_text = String::new();
                            let mut done = false;
                            let mut err_msg: Option<String> = None;
                            for ev in context.ai_events.iter() {
                                match ev {
                                    AiResultEvent::StreamChunk { id, text, done: d } if id == &req_id => {
                                        full_text.push_str(text);
                                        if *d {
                                            done = true;
                                        }
                                    }
                                    AiResultEvent::Success { id, result } if id == &req_id => {
                                        full_text = result.clone();
                                        done = true;
                                    }
                                    AiResultEvent::Error { id, message } if id == &req_id => {
                                        err_msg = Some(message.clone());
                                        done = true;
                                    }
                                    _ => {}
                                }
                            }
                            if done {
                                got = Some((full_text, err_msg));
                            }
                        }
                        crate::tabs_system::node_editor::state::CopilotBackend::Gemini => {
                            if let Some(host) = context.gemini_copilot_host {
                                for r in host.poll() {
                                    if r.id == req_id {
                                        got = Some((r.text, r.error));
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    let Some((text, err)) = got else { continue; };
                    if self.copilot_relays_cancelled.contains(&req_id) {
                        continue;
                    }
                    let Some(s) = self.copilot_relays.get_mut(&sid) else { continue; };
                    if text.trim().is_empty() {
                        s.status = crate::tabs_system::node_editor::state::CopilotRelayStatus::Error;
                        s.error = err.or_else(|| Some("Empty response.".to_string()));
                        continue;
                    }
                    fn parse_ghost(raw: &str) -> (Vec<String>, Option<String>, Option<String>, std::collections::HashMap<String, String>) {
                        use serde_json::Value;
                        let Some(v) = parse_first_json_value(raw) else { return (parse_nodes_fallback(raw), None, None, std::collections::HashMap::new()); };
                        if let Value::Array(a) = v {
                            let nodes = a.into_iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect::<Vec<_>>();
                            return (nodes, None, None, std::collections::HashMap::new());
                        }
                        let Value::Object(o) = v else { return (Vec::new(), None, None, std::collections::HashMap::new()); };
                        let nodes = o.get("nodes").and_then(|n| n.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect::<Vec<_>>()).unwrap_or_default();
                        let rt = o.get("reason_title").and_then(|x| x.as_str()).map(|s| s.to_string());
                        let r = o.get("reason").and_then(|x| x.as_str()).map(|s| s.to_string());
                        let params = o.get("params").and_then(|p| p.as_object()).map(|m| {
                            m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))
                                .or_else(|| v.as_bool().map(|b| (k.clone(), b.to_string())))
                                .or_else(|| v.as_i64().map(|i| (k.clone(), i.to_string())))
                            ).collect()
                        }).unwrap_or_default();
                        (nodes, rt, r, params)
                    }
                    let (names, reason_title, reason, params) = parse_ghost(&text);
                    if names.is_empty() {
                        s.status = crate::tabs_system::node_editor::state::CopilotRelayStatus::Error;
                        s.error = Some("Parse failed.".to_string());
                        continue;
                    }
                    s.reason_title = reason_title;
                    s.reason = reason;
                    s.ghost_params = params;
                    let sz = crate::node_editor_settings::resolved_node_size(context.node_editor_settings);
                    let stride = (sz[0] + 80.0).max(220.0);
                    let knowledge = crate::libs::ai_service::native_tiny_model::knowledge_cache::GLOBAL_KNOWLEDGE_CACHE
                        .get()
                        .map(|k| k.nodes.clone());
                    let mut nodes = Vec::new();
                    let mut links = Vec::new();
                    let mut prev: Option<crate::nodes::NodeId> = None;
                    for (i, name) in names.into_iter().take(target_nodes.max(1).min(32)).enumerate() {
                        let id = crate::nodes::NodeId::new_v4();
                        let pos = egui::pos2(anchor.x + i as f32 * stride, anchor.y);
                        let (inputs, input_style) = if let Some(map) = knowledge.as_ref() {
                            if let Some(k) = map.get(&name) {
                                let ins: Vec<crate::nodes::PortId> = k
                                    .io
                                    .inputs
                                    .iter()
                                    .map(|p| crate::nodes::PortId::from(p.name.as_str()))
                                    .collect();
                                let style = if ins.len() > 1 || k.io.input_type == "Multi" {
                                    crate::nodes::InputStyle::Collection
                                } else {
                                    crate::nodes::InputStyle::Individual
                                };
                                (ins, style)
                            } else {
                                (Vec::new(), crate::nodes::InputStyle::Individual)
                            }
                        } else {
                            (Vec::new(), crate::nodes::InputStyle::Individual)
                        };
                        let outputs = if let Some(map) = knowledge.as_ref() {
                            map.get(&name)
                                .map(|k| {
                                    k.io
                                        .outputs
                                        .iter()
                                        .map(|p| crate::nodes::PortId::from(p.name.as_str()))
                                        .collect()
                                })
                                .unwrap_or_else(|| vec![crate::nodes::PortId::from("Output")])
                        } else {
                            vec![crate::nodes::PortId::from("Output")]
                        };
                        nodes.push(crate::tabs_system::node_editor::state::NodeSnapshot {
                            id,
                            name,
                            position: pos,
                            size: egui::vec2(sz[0], sz[1]),
                            header_h: context.node_editor_settings.node_header_h_base.max(10.0),
                            input_style,
                            style: crate::nodes::NodeStyle::Normal,
                            is_template: false,
                            is_bypassed: false,
                            is_display_node: false,
                            is_locked: false,
                            inputs,
                            outputs,
                        });
                        if let Some(p) = prev {
                            links.push((p, id));
                        }
                        prev = Some(id);
                    }
                    s.ghost = Some(crate::tabs_system::node_editor::state::GhostGraph { nodes, links });
                    s.status = crate::tabs_system::node_editor::state::CopilotRelayStatus::Ready;
                }
            }

            // Execute queued relay actions (from overlay UI / shortcuts).
            if !self.copilot_relay_actions.is_empty() {
                let actions = std::mem::take(&mut self.copilot_relay_actions);
                for a in actions {
                    let Some(mut s) = self.copilot_relays.get(&a.session_id).cloned() else { continue; };
                    match a.kind {
                        crate::tabs_system::node_editor::state::CopilotRelayActionKind::Cancel => {
                            self.copilot_relays_cancelled.insert(s.request_id.clone());
                            self.copilot_relays.remove(&a.session_id);
                            if self.copilot_relay_selected == Some(a.session_id) {
                                self.copilot_relay_selected = None;
                            }
                        }
                        crate::tabs_system::node_editor::state::CopilotRelayActionKind::Reroll => {
                            let Some(mut s0) = self.copilot_relays.remove(&a.session_id) else { continue; };
                            self.copilot_relays_cancelled.insert(s0.request_id.clone());
                            s0.request_id = format!("relay_{}", uuid::Uuid::new_v4());
                            s0.status = crate::tabs_system::node_editor::state::CopilotRelayStatus::Generating;
                            s0.ghost = None;
                            s0.error = None;
                            crate::tabs_system::node_editor::events::common::request_relay_session(&mut s0, self, context);
                            self.copilot_relays.insert(a.session_id, s0);
                        }
                        crate::tabs_system::node_editor::state::CopilotRelayActionKind::Apply => {
                            if s.status != crate::tabs_system::node_editor::state::CopilotRelayStatus::Ready {
                                continue;
                            }
                            let Some(ghost) = s.ghost.take() else { continue; };
                            let root_graph = &mut context.node_graph_res.0;
                            crate::tabs_system::node_editor::cda::navigation::with_current_graph_mut(
                                root_graph,
                                &self.cda_state,
                                |node_graph| {
                                    use crate::cunning_core::command::basic::{
                                        CmdAddNetworkBox, CmdAddNode, CmdAddStickyNote, CmdBatch, CmdSetConnection, CmdSetDisplayNode,
                                    };
                                    let mut cmds: Vec<Box<dyn crate::cunning_core::command::Command>> = Vec::new();
                                    let mut created: Vec<crate::nodes::Node> = Vec::new();
                                    for snap in &ghost.nodes {
                                        let mut node = crate::ui::prepare_generic_node(
                                            context.node_registry,
                                            context.node_editor_settings,
                                            snap.position,
                                            &snap.name,
                                        );
                                        for (key, val) in &s.ghost_params {
                                            if let Some((node_name, param_name)) = key.split_once('.') {
                                                if node_name == snap.name {
                                                    if let Some(p) = node.parameters.iter_mut().find(|p| p.name == param_name) {
                                                        use crate::cunning_core::traits::parameter::{ParameterUIType, ParameterValue};
                                                        match (&mut p.value, &p.ui_type) {
                                                            (ParameterValue::Bool(_), _) => p.value = ParameterValue::Bool(val.parse().unwrap_or(false)),
                                                            (ParameterValue::String(_), _) => p.value = ParameterValue::String(val.clone()),
                                                            (ParameterValue::Int(_), ParameterUIType::Dropdown { choices }) => {
                                                                if let Some((_, out_v)) = choices.iter().find(|(label, _)| label.eq_ignore_ascii_case(val)) {
                                                                    p.value = ParameterValue::Int(*out_v);
                                                                }
                                                            }
                                                            (ParameterValue::Int(_), _) => {
                                                                if let Ok(i) = val.parse() {
                                                                    p.value = ParameterValue::Int(i);
                                                                }
                                                            }
                                                            _ => {}
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        created.push(node);
                                    }
                                    // Paper-like AI layout: NetworkBox(title) -> Sticky(content) -> Nodes(grid).
                                    let module_title = s
                                        .reason_title
                                        .clone()
                                        .unwrap_or_else(|| "AI Module".to_string());
                                    let reason = wrap_text(
                                        s.reason.clone().unwrap_or_default().trim(),
                                        context.node_editor_settings.ai_layout_sticky_wrap_cols as usize,
                                    );
                                    let mut note = if reason.is_empty() {
                                        None
                                    } else {
                                        let sz = crate::node_editor_settings::resolved_node_size(context.node_editor_settings);
                                        let gap_x = context.node_editor_settings.ai_layout_gap_x.max(16.0);
                                        let max_cols = context.node_editor_settings.ai_layout_max_cols.max(1) as usize;
                                        let cols = created.len().min(max_cols).max(1);
                                        let nodes_w = cols as f32 * sz[0] + (cols.saturating_sub(1) as f32) * gap_x;
                                        let w = nodes_w
                                            .max(context.node_editor_settings.ai_layout_sticky_min_w)
                                            .min(context.node_editor_settings.ai_layout_sticky_max_w);
                                        let lines = reason.lines().count().max(3) as f32;
                                        let h = context
                                            .node_editor_settings
                                            .ai_layout_sticky_min_h
                                            .max(lines * context.node_editor_settings.ai_layout_sticky_line_h + 34.0);
                                        let rgba = context.node_editor_settings.ai_layout_sticky_rgba;
                                        Some(crate::nodes::StickyNote {
                                            id: crate::nodes::StickyNoteId::new_v4(),
                                            rect: Rect::from_min_size(s.anchor_graph_pos, egui::vec2(w, h)),
                                            title: String::new(),
                                            content: reason,
                                            color: Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]),
                                        })
                                    };
                                    let placed = layout_ai_module(
                                        node_graph,
                                        context.node_editor_settings,
                                        s.anchor_graph_pos,
                                        &mut created,
                                        note.as_mut(),
                                    );
                                    for node in created.iter().cloned() {
                                        cmds.push(Box::new(CmdAddNode { node }));
                                    }
                                    if let Some(first) = created.first() {
                                        if let Some(in_port) = first.inputs.keys().next().cloned() {
                                            for (i, (from_node, from_port)) in s.sources.iter().cloned().enumerate() {
                                                let conn = crate::nodes::Connection {
                                                    id: crate::nodes::ConnectionId::new_v4(),
                                                    from_node,
                                                    from_port,
                                                    to_node: first.id,
                                                    to_port: in_port.clone(),
                                                    order: i as i32,
                                                    waypoints: Vec::new(),
                                                };
                                                cmds.push(Box::new(CmdSetConnection::new(conn, i == 0)));
                                            }
                                        }
                                    }
                                    for w in created.windows(2) {
                                        let a = &w[0];
                                        let b = &w[1];
                                        let out_port = a
                                            .outputs
                                            .keys()
                                            .next()
                                            .cloned()
                                            .unwrap_or_else(|| crate::nodes::PortId::from("Output"));
                                        let in_port = b
                                            .inputs
                                            .keys()
                                            .next()
                                            .cloned()
                                            .unwrap_or_else(|| crate::nodes::PortId::from("Input"));
                                        let conn = crate::nodes::Connection {
                                            id: crate::nodes::ConnectionId::new_v4(),
                                            from_node: a.id,
                                            from_port: out_port,
                                            to_node: b.id,
                                            to_port: in_port,
                                            order: 0,
                                            waypoints: Vec::new(),
                                        };
                                        cmds.push(Box::new(CmdSetConnection::new(conn, true)));
                                    }
                                    if let Some(last) = created.last() {
                                        cmds.push(Box::new(CmdSetDisplayNode::new(
                                            node_graph.display_node,
                                            Some(last.id),
                                        )));
                                    }
                                    if let Some(note) = &note {
                                        cmds.push(Box::new(CmdAddStickyNote { note: note.clone() }));
                                    }
                                    let mut nodes_inside = std::collections::HashSet::new();
                                    for n in &created { nodes_inside.insert(n.id); }
                                    let mut stickies_inside = std::collections::HashSet::new();
                                    if let Some(note) = &note { stickies_inside.insert(note.id); }
                                    let rgba = context.node_editor_settings.ai_layout_box_rgba;
                                    let box_ = crate::nodes::NetworkBox {
                                        id: crate::nodes::NetworkBoxId::new_v4(),
                                        rect: placed,
                                        title: module_title,
                                        color: Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]),
                                        nodes_inside,
                                        stickies_inside,
                                    };
                                    cmds.push(Box::new(CmdAddNetworkBox { box_ }));
                                    if !cmds.is_empty() {
                                        context
                                            .node_editor_state
                                            .execute(Box::new(CmdBatch::new("AI Relay Apply", cmds)), node_graph);
                                        context.graph_changed_writer.write_default();
                                    }
                                },
                            );
                            self.copilot_relays_cancelled.insert(s.request_id.clone());
                            self.copilot_relays.remove(&a.session_id);
                            if self.copilot_relay_selected == Some(a.session_id) {
                                self.copilot_relay_selected = None;
                            }
                        }
                    }
                }
            }

            if let Some(req_id) = self.box_note_request_id.clone() {
                let backend = self.box_note_inflight_backend.unwrap_or(self.copilot_backend);
                let mut got: Option<(String, Option<String>)> = None;
                match backend {
                    crate::tabs_system::node_editor::state::CopilotBackend::LocalTiny => {
                        if let Some(host) = context.tiny_model_host { for r in host.poll() { if r.id == req_id { got = Some((r.text, r.error)); break; } } }
                    }
                    crate::tabs_system::node_editor::state::CopilotBackend::LocalThink => {
                        use crate::libs::ai_service::native_candle::AiResultEvent;
                        let mut full_text = String::new();
                        let mut done = false;
                        let mut err_msg: Option<String> = None;
                        for ev in context.ai_events.iter() {
                            match ev {
                                AiResultEvent::StreamChunk { id, text, done: d } if id == &req_id => { full_text.push_str(text); if *d { done = true; } }
                                AiResultEvent::Success { id, result } if id == &req_id => { full_text = result.clone(); done = true; }
                                AiResultEvent::Error { id, message } if id == &req_id => { err_msg = Some(message.clone()); done = true; }
                                _ => {}
                            }
                        }
                        if done { got = Some((full_text, err_msg)); }
                    }
                    crate::tabs_system::node_editor::state::CopilotBackend::Gemini => {
                        if let Some(host) = context.gemini_copilot_host { for r in host.poll() { if r.id == req_id { got = Some((r.text, r.error)); break; } } }
                    }
                }
                if let Some((text, _err)) = got {
                    self.box_note_request_id = None;
                    self.box_note_inflight_backend = None;
                    fn parse_note(raw: &str) -> Option<(String, String)> {
                        let s = raw.trim();
                        let v: serde_json::Value = serde_json::from_str(s).ok()?;
                        let o = v.as_object()?;
                        let title = o.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        let content = o.get("content").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        if title.is_empty() && content.is_empty() { return None; }
                        Some((title, content))
                    }
                    let Some((title, content)) = parse_note(&text) else { return; };
                    let nodes = self.box_note_pending_nodes.clone();
                    let stickies = self.box_note_pending_stickies.clone();
                    self.box_note_pending_nodes.clear();
                    self.box_note_pending_stickies.clear();
                    let root_graph = &mut context.node_graph_res.0;
                    cda::navigation::with_current_graph_mut(root_graph, &self.cda_state, |g| {
                        let mut min_pos = egui::Pos2::new(f32::MAX, f32::MAX);
                        let mut max_pos = egui::Pos2::new(f32::MIN, f32::MIN);
                        for id in &nodes {
                            if let Some(n) = g.nodes.get(id) {
                                min_pos.x = min_pos.x.min(n.position.x);
                                min_pos.y = min_pos.y.min(n.position.y);
                                max_pos.x = max_pos.x.max(n.position.x + n.size.x);
                                max_pos.y = max_pos.y.max(n.position.y + n.size.y);
                            }
                        }
                        for id in &stickies {
                            if let Some(s) = g.sticky_notes.get(id) {
                                min_pos.x = min_pos.x.min(s.rect.min.x);
                                min_pos.y = min_pos.y.min(s.rect.min.y);
                                max_pos.x = max_pos.x.max(s.rect.max.x);
                                max_pos.y = max_pos.y.max(s.rect.max.y);
                            }
                        }
                        if min_pos.x == f32::MAX { return; }
                        let pad = egui::Vec2::new(40.0, 40.0);
                        let box_rect = egui::Rect::from_min_max(min_pos - pad, max_pos + pad);
                        let note_id = crate::nodes::StickyNoteId::new_v4();
                        let note_rect = crate::tabs_system::node_editor::events::common::auto_place_note_rect(
                            g,
                            egui::Rect::from_min_size(
                                box_rect.min + egui::Vec2::new(10.0, 10.0),
                                egui::Vec2::new(220.0, 120.0),
                            ),
                        );
                        let note = crate::nodes::StickyNote { id: note_id, rect: note_rect, title, content, color: Color32::from_rgb(255, 243, 138) };
                        let mut nodes_inside = std::collections::HashSet::new();
                        for id in &nodes { nodes_inside.insert(*id); }
                        let mut stickies_inside = std::collections::HashSet::new();
                        for id in &stickies { stickies_inside.insert(*id); }
                        stickies_inside.insert(note_id);
                        let nbox = crate::nodes::NetworkBox { id: crate::nodes::NetworkBoxId::new_v4(), rect: box_rect, title: "Network Box".to_string(), color: Color32::from_rgba_unmultiplied(50, 50, 80, 100), nodes_inside, stickies_inside };
                        let cmds: Vec<Box<dyn crate::cunning_core::command::Command>> = vec![
                            Box::new(CmdAddStickyNote { note }),
                            Box::new(CmdAddNetworkBox { box_: nbox }),
                        ];
                        context.node_editor_state.execute(Box::new(CmdBatch::new("Annotate Selection", cmds)), g);
                        context.graph_changed_writer.write_default();
                    });
                }
            }

            // Poll graph explain results
            if let Some(req_id) = self.explain_request_id.clone() {
                let backend = self.explain_inflight_backend.unwrap_or(self.copilot_backend);
                let mut got: Option<(String, Option<String>)> = None;
                match backend {
                    crate::tabs_system::node_editor::state::CopilotBackend::LocalTiny => {
                        if let Some(host) = context.tiny_model_host { for r in host.poll() { if r.id == req_id { got = Some((r.text, r.error)); break; } } }
                    }
                    crate::tabs_system::node_editor::state::CopilotBackend::LocalThink => {
                        use crate::libs::ai_service::native_candle::AiResultEvent;
                        let mut full = String::new(); let mut done = false;
                        for ev in context.ai_events.iter() {
                            match ev {
                                AiResultEvent::StreamChunk { id, text, done: d } if id == &req_id => { full.push_str(text); if *d { done = true; } }
                                AiResultEvent::Success { id, result } if id == &req_id => { full = result.clone(); done = true; }
                                AiResultEvent::Error { id, .. } if id == &req_id => { done = true; }
                                _ => {}
                            }
                        }
                        if done { got = Some((full, None)); }
                    }
                    crate::tabs_system::node_editor::state::CopilotBackend::Gemini => {
                        if let Some(host) = context.gemini_copilot_host { for r in host.poll() { if r.id == req_id { got = Some((r.text, r.error)); break; } } }
                    }
                }
                if let Some((text, err)) = got {
                    if text.trim().is_empty()
                        && backend == crate::tabs_system::node_editor::state::CopilotBackend::Gemini
                        && err.as_deref().is_some_and(|e| e.contains("HTTP 429") || e.contains("RESOURCE_EXHAUSTED"))
                    {
                        self.explain_request_id = None;
                        self.explain_inflight_backend = None;
                        self.explain_pending_nodes.clear();
                        self.explain_result = Some((
                            "Gemini unavailable".to_string(),
                            "Cloud quota/rate limit. Try LocalTiny/LocalThink.".to_string(),
                        ));
                        return;
                    }
                    self.explain_request_id = None;
                    self.explain_inflight_backend = None;
                    self.explain_pending_nodes.clear();
                    fn parse_explain(raw: &str) -> Option<(String, String)> {
                        let v: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;
                        let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        let exp = v.get("explanation").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        if title.is_empty() && exp.is_empty() { None } else { Some((title, exp)) }
                    }
                    self.explain_result = parse_explain(&text);
                }
            }

            // Poll Coverlay Panel generation (Gemini)
            if let Some(req_id) = self.coverlay_gen_request_id.clone() {
                if let Some(host) = context.gemini_copilot_host {
                    let mut got: Option<(String, Option<String>)> = None;
                    for r in host.poll() {
                        if r.id == req_id {
                            got = Some((r.text, r.error));
                            break;
                        }
                    }
                    if let Some((text, err)) = got {
                        self.coverlay_gen_request_id = None;
                        if text.trim().is_empty() {
                            self.coverlay_gen_error = Some(err.unwrap_or_else(|| "Empty response.".to_string()));
                        } else {
                            let parse_json = |s: &str| -> Option<serde_json::Value> {
                                serde_json::from_str::<serde_json::Value>(s.trim())
                                    .ok()
                                    .or_else(|| {
                                        let (l, r) = (s.find('{')?, s.rfind('}')?);
                                        if l < r { serde_json::from_str::<serde_json::Value>(&s[l..=r]).ok() } else { None }
                                    })
                            };
                            if let Some(v) = parse_json(&text) {
                                let json = serde_json::to_string_pretty(&v).unwrap_or_else(|_| text.trim().to_string());
                                let sel: Vec<crate::nodes::NodeId> = context.ui_state.selected_nodes.iter().copied().collect();
                                let anchor_screen = self.coverlay_gen_anchor;
                                let pan = self.pan;
                                let zoom = self.zoom;
                                let root = &mut context.node_graph_res.0;
                                cda::navigation::with_current_graph_mut(root, &self.cda_state, |g| {
                                    let mut coverlay_id: Option<crate::nodes::NodeId> = None;
                                    for id in &sel {
                                        if let Some(n) = g.nodes.get(id) {
                                            if matches!(&n.node_type, NodeType::Generic(s) if s.trim() == "Coverlay Panel") {
                                                coverlay_id = Some(*id);
                                                break;
                                            }
                                        }
                                    }
                                    let pos = if let Some(id) = coverlay_id.or(sel.first().copied()) {
                                        g.nodes.get(&id).map(|n| n.position).unwrap_or(egui::Pos2::new(100.0, 100.0))
                                    } else if let Some(p) = anchor_screen {
                                        ((p - editor_rect.min - pan) / zoom).to_pos2()
                                    } else {
                                        egui::Pos2::new(100.0, 100.0)
                                    };
                                    let id = coverlay_id.unwrap_or_else(|| {
                                        let node = crate::ui::prepare_generic_node(context.node_registry, context.node_editor_settings, pos, "Coverlay Panel");
                                        let id = node.id;
                                        context.node_editor_state.execute(Box::new(crate::cunning_core::command::basic::CmdAddNode { node }), g);
                                        id
                                    });
                                    if let Some(n) = g.nodes.get_mut(&id) {
                                        if let Some(p) = n.parameters.iter_mut().find(|p| p.name == "bindings_json") {
                                            p.value = crate::nodes::parameter::ParameterValue::String(json.clone());
                                        } else {
                                            n.parameters.push(crate::nodes::parameter::Parameter::new(
                                                "bindings_json",
                                                "Bindings (Internal)",
                                                "Internal",
                                                crate::nodes::parameter::ParameterValue::String(json.clone()),
                                                crate::nodes::parameter::ParameterUIType::Code,
                                            ));
                                        }
                                        g.mark_dirty(id);
                                    }
                                    context.ui_state.selected_nodes.clear();
                                    context.ui_state.selected_nodes.insert(id);
                                    context.ui_state.last_selected_node_id = Some(id);
                                });
                                context.graph_changed_writer.write_default();
                                self.coverlay_gen_error = None;
                            } else {
                                self.coverlay_gen_error = Some("Failed to parse JSON.".to_string());
                            }
                        }
                    }
                } else {
                    self.coverlay_gen_request_id = None;
                    self.coverlay_gen_error = Some("Gemini backend selected but gemini_copilot_host is None".to_string());
                }
            }

            // Draw explain tooltip if result available
            if let (Some((title, exp)), Some(pos)) = (&self.explain_result, self.explain_show_pos) {
                let painter = ui.painter();
                let font = egui::FontId::proportional(13.0);
                let max_w = 280.0;
                let title_galley = painter.layout(title.clone(), egui::FontId::proportional(14.0), Color32::WHITE, max_w);
                let exp_galley = painter.layout(exp.clone(), font.clone(), Color32::from_gray(220), max_w);
                let pad = 10.0;
                let h = title_galley.size().y + exp_galley.size().y + pad * 3.0;
                let w = title_galley.size().x.max(exp_galley.size().x) + pad * 2.0;
                let rect = egui::Rect::from_min_size(pos, egui::Vec2::new(w, h));
                painter.rect_filled(rect, 6.0, Color32::from_rgba_unmultiplied(30, 30, 40, 240));
                painter.rect_stroke(
                    rect,
                    6.0,
                    egui::Stroke::new(1.0, Color32::from_gray(80)),
                    egui::StrokeKind::Inside,
                );
                painter.galley(rect.min + egui::Vec2::new(pad, pad), title_galley, Color32::WHITE);
                painter.galley(rect.min + egui::Vec2::new(pad, pad + 20.0), exp_galley, Color32::from_gray(220));
                // Click outside to dismiss
                if ui.input(|i| i.pointer.any_click()) && !rect.contains(ui.input(|i| i.pointer.interact_pos().unwrap_or_default())) {
                    self.explain_result = None;
                    self.explain_show_pos = None;
                }
            }

            // Ghost overlay (nodes + links).
            if let Some(ghost) = &self.ghost_graph {
                drawing::paint_nodes_retained(ui, self, context, editor_rect, &ghost.nodes, 0.5, None);
                drawing::draw_ghost_links(ui, self, ghost, editor_rect, 0.5);
                drawing::draw_ghost_reason_note(ui, self, ghost, editor_rect);
            }

            crate::tabs_system::node_editor::events::common::handle_connection_drop(self, ui, context, editor_rect);
            handle_menus(self, ui, context, &response, editor_rect);
            handle_cut_tool(self, ui, context, editor_rect);

            // Draw HUD (Always on top)
            ui.allocate_ui_at_rect(editor_rect, |ui| {
                ui.push_id(("ne_hud", editor_rect_id), |ui| {
                    hud::draw_hud(ui, self, context, editor_rect);
                });
            });

            let edit_stack = self.cda_state.edit_stack.clone();
            let breadcrumb: Vec<crate::nodes::NodeId> = edit_stack.into_iter().map(|l| l.cda_node_id).collect();
            context.node_editor_state.cda_path = breadcrumb.clone();
            handle_cda_actions(self, ui, context);
            {
                let cda_state = &mut self.cda_state;
                let root_graph = &mut context.node_graph_res.0;
                cda::navigation::with_graph_by_path_mut(root_graph, &breadcrumb, |g| {
                    cda::property_window::draw_property_window(
                        ui.ctx(),
                        g,
                        cda_state,
                        context.node_editor_settings,
                        context.node_registry,
                    );
                });
            }
            cda::create_dialog::draw_create_dialog(ui.ctx(), &mut self.cda_state, context);

            // Coverlay Panel generator prompt
            if self.coverlay_gen_open {
                let mut open = true;
                let anchor = self.coverlay_gen_anchor.unwrap_or_else(|| editor_rect.center());
                egui::Window::new("Generate Coverlay Panel")
                    .open(&mut open)
                    .fixed_pos(anchor + egui::vec2(12.0, 12.0))
                    .resizable(false)
                    .show(ui.ctx(), |ui| {
                        ui.label("Instruction (Gemini):");
                        ui.text_edit_multiline(&mut self.coverlay_gen_prompt);
                        if let Some(e) = &self.coverlay_gen_error {
                            ui.colored_label(Color32::LIGHT_RED, e);
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Generate").clicked() {
                                self.coverlay_gen_open = false;
                                request_coverlay_panel_generate(self, context);
                            }
                            if ui.button("Cancel").clicked() {
                                self.coverlay_gen_open = false;
                            }
                        });
                    });
                if !open {
                    self.coverlay_gen_open = false;
                }
            }

            // Connection hint removed (feature disabled).
        });
    }

    fn title(&self) -> egui::WidgetText {
        "Node Editor".into()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn is_immediate(&self) -> bool {
        false
    }

    fn retained_key(&self, ui: &egui::Ui, context: &EditorTabContext) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();

        // 1. Data Revision
        context.graph_revision.hash(&mut hasher);
        self.geometry_rev.hash(&mut hasher); // Important: Local geometry revision

        // 2. View State
        context.node_editor_state.pan.x.to_bits().hash(&mut hasher);
        context.node_editor_state.pan.y.to_bits().hash(&mut hasher);
        context.node_editor_state.zoom.to_bits().hash(&mut hasher);

        // 3. Selection
        context.ui_state.selected_nodes.len().hash(&mut hasher);
        for id in &context.ui_state.selected_nodes {
            id.hash(&mut hasher);
        }
        context
            .ui_state
            .selected_connections
            .len()
            .hash(&mut hasher);
        for id in &context.ui_state.selected_connections {
            id.hash(&mut hasher);
        }

        // 3.5 CDA Path (supports infinite nesting without hardcoding)
        self.cda_state.depth().hash(&mut hasher);
        for id in self.cda_state.breadcrumb() {
            id.hash(&mut hasher);
        }

        // 4. Interaction (Active / Hovering)
        // If the mouse is moving over the editor, we rebuild to support hover highlights/tooltips.
        // This is "Immediate-like behavior" only when interacting, satisfying "Idle 0".
        // Using `available_rect` from `ui` context might be tricky as `retained_key` is called before layout?
        // But `TabUi` mixes `available_rect` into the key anyway.
        // So we just need to know if the mouse is *inside* that rect?
        // Since we don't know the rect yet, we simply check if pointer is active or moving.
        // If pointer is moving, we rebuild. This is cheap enough for the "Shell", and heavy inner parts are retained.

        if let Some(pos) = ui.input(|i| i.pointer.latest_pos()) {
            // Hash position to detect movement
            pos.x.to_bits().hash(&mut hasher);
            pos.y.to_bits().hash(&mut hasher);
        }
        ui.input(|i| i.pointer.any_down()).hash(&mut hasher);

        hasher.finish()
    }
}

// Node info is now hosted in a native Bevy window via `node_info_tab::NodeInfoTab`.
