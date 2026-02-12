use crate::cunning_core::command::basic::{
    CmdAddNetworkBox, CmdAddNode, CmdAddStickyNote, CmdBatch, CmdSetConnection,
};
use crate::invalidator::RepaintCause;
use crate::nodes::{
    Connection, ConnectionId, InputStyle, NetworkBox, NetworkBoxId, StickyNote, StickyNoteId,
};
use crate::tabs_system::node_editor::bars::items::promote_note;
use crate::tabs_system::node_editor::cda;
use crate::tabs_system::node_editor::NodeEditorTab;
use crate::tabs_system::node_editor::state::CopilotRelaySession;
use crate::tabs_system::EditorTabContext;
use crate::ui::prepare_generic_node;
use bevy_egui::egui::{self, Color32, Pos2, Rect, Vec2};
use cunning_cda_runtime::registry::{MultiWirePolicy, RuntimeRegistry as CdaRtRegistry};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

fn effective_backend(
    editor: &NodeEditorTab,
) -> crate::tabs_system::node_editor::state::CopilotBackend {
    use crate::tabs_system::node_editor::state::CopilotBackend;
    let b = editor.copilot_backend;
    if b == CopilotBackend::Gemini {
        if let Some(until) = editor.copilot_cloud_disabled_until {
            if Instant::now() < until {
                return CopilotBackend::LocalTiny;
            }
        }
    }
    b
}

fn consume_selected_promote_notes(editor: &NodeEditorTab, context: &mut EditorTabContext) -> String {
    if context.ui_state.selected_promote_notes.is_empty() { return String::new(); }
    let breadcrumb = editor.cda_state.breadcrumb();
    let selected: Vec<_> = context.ui_state.selected_promote_notes.iter().copied().collect();
    let mut hint = String::new();
    let root = &mut context.node_graph_res.0;
    cda::navigation::with_graph_by_path_mut(root, &breadcrumb, |g| {
        for id in selected {
            let Some(note) = g.promote_notes.get_mut(&id) else { continue; };
            let t = note.content.trim();
            if t.is_empty() || note.used_at.is_some() { continue; }
            hint.push_str("[UserIntent]\n"); hint.push_str(t); hint.push_str("\n\n");
            note.used_at = Some(Instant::now());
        }
    });
    hint
}

pub fn request_relay_session(
    session: &mut CopilotRelaySession,
    editor: &NodeEditorTab,
    context: &mut EditorTabContext,
) {
    if session.sources.is_empty() {
        session.status = crate::tabs_system::node_editor::state::CopilotRelayStatus::Error;
        session.error = Some("Missing sources.".to_string());
        return;
    }
    let (start_node, _start_port) = session.sources[0];
    let start_name = editor
        .cached_nodes
        .iter()
        .find(|n| n.id == start_node)
        .map(|n| n.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    let kc_ref = crate::libs::ai_service::native_tiny_model::knowledge_cache::GLOBAL_KNOWLEDGE_CACHE.get();
    let knowledge_filtered = if let Some(ref kc) = kc_ref {
        let start_cat = kc.get_category(&start_name).unwrap_or("Geometry");
        let related = crate::libs::ai_service::native_tiny_model::knowledge_cache::KnowledgeCache::related_categories(start_cat);
        kc.build_filtered_prompt(&related, Some(&start_name))
    } else {
        crate::libs::ai_service::native_tiny_model::knowledge_cache::GLOBAL_KNOWLEDGE_PROMPT_BLOB
            .get()
            .cloned()
            .unwrap_or_default()
    };
    let target_hint = if session.target_nodes > 0 {
        format!("\n\nTarget length: about {} nodes (±2).", session.target_nodes)
    } else {
        String::new()
    };

    let promote_hint = consume_selected_promote_notes(editor, context);
    let mut sources_block = String::new();
    sources_block.push_str(&format!("[Sources]\ncount={}\n", session.sources.len()));
    for (nid, port) in session.sources.iter().copied() {
        let name = editor
            .cached_nodes
            .iter()
            .find(|n| n.id == nid)
            .map(|n| n.name.as_str())
            .unwrap_or("Unknown");
        sources_block.push_str(&format!("- node={} port={}\n", name, port));
    }
    if session.sources.len() > 1 {
        sources_block.push_str("\nIf multiple sources, ensure the first suggested node can accept multiple inputs (or introduce an aggregator first).\n");
    }

    let prompt = format!(
        "<|im_start|>system\nYou are Cunning3D Copilot. Return ONLY valid JSON.\nFormat: {{\"nodes\":[\"A\",\"B\"],\"params\":{{\"A.param\":\"val\"}},\"reason_title\":\"...\",\"reason\":\"...\"}}\nRules:\n- nodes must be known Cunning3D nodes from the knowledge below\n- reason <= 3 lines\n- params optional; avoid numeric params unless it's dropdown label\n<|im_end|>\n<|im_start|>user\n{promote}{kb}\n\n[Start]\nnode={name}\n\n{sources}{target}\n\nSuggest next nodes.\n<|im_end|>\n<|im_start|>assistant\n",
        promote = promote_hint,
        kb = knowledge_filtered,
        name = start_name,
        sources = sources_block,
        target = target_hint
    );
    match session.backend {
        crate::tabs_system::node_editor::state::CopilotBackend::LocalTiny => {
            let Some(host) = context.tiny_model_host else {
                session.status = crate::tabs_system::node_editor::state::CopilotRelayStatus::Error;
                session.error = Some("LocalTiny host missing.".to_string());
                return;
            };
            host.request(&session.request_id, &prompt);
        }
        crate::tabs_system::node_editor::state::CopilotBackend::LocalThink => {
            let Some(host) = context.native_ai_host else {
                session.status = crate::tabs_system::node_editor::state::CopilotRelayStatus::Error;
                session.error = Some("LocalThink host missing.".to_string());
                return;
            };
            host.request_prediction(session.request_id.clone(), prompt, 512);
        }
        crate::tabs_system::node_editor::state::CopilotBackend::Gemini => {
            let Some(host) = context.gemini_copilot_host else {
                session.status = crate::tabs_system::node_editor::state::CopilotRelayStatus::Error;
                session.error = Some("Gemini host missing.".to_string());
                return;
            };
            host.request_with_model(&session.request_id, &prompt, Some(editor.gemini_cloud_model.model_name()));
        }
    }
}

pub fn handle_pan_zoom(
    _editor: &mut NodeEditorTab,
    ui: &mut egui::Ui,
    context: &mut EditorTabContext,
    response: &egui::Response,
    editor_rect: Rect,
) {
    let mut changed = false;
    let popup_open = ui.ctx().memory(|m| {
        #[allow(deprecated)]
        {
            m.any_popup_open()
        }
    });
    // Multi-touch
    if let Some(multi_touch) = ui.input(|i| i.multi_touch()) {
        context.node_editor_state.target_pan += multi_touch.translation_delta;
        if multi_touch.translation_delta != Vec2::ZERO {
            changed = true;
        }
        let zoom_delta = multi_touch.zoom_delta;
        if zoom_delta != 1.0 {
            let zoom_center = ui
                .input(|i| i.pointer.interact_pos())
                .unwrap_or(editor_rect.center());
            let local_pinch_center = zoom_center - editor_rect.min;
            let old_zoom = context.node_editor_state.target_zoom;
            let new_zoom = (old_zoom * zoom_delta).clamp(0.1, 5.0);
            let effective_zoom_delta = new_zoom / old_zoom;
            context.node_editor_state.target_zoom = new_zoom;
            let vec = context.node_editor_state.target_pan - local_pinch_center;
            context.node_editor_state.target_pan = local_pinch_center + vec * effective_zoom_delta;
            changed = true;
        }
    }

    // Mouse (disable while popups are open so context menus/search menus don't fight with pan/zoom).
    if !popup_open && response.dragged_by(egui::PointerButton::Middle) {
        context.node_editor_state.target_pan += response.drag_delta();
        if response.drag_delta() != Vec2::ZERO {
            changed = true;
        }
    }

    // Mouse wheel zoom (cursor-centered). Gate on popups to prevent scroll/menu flicker.
    let scroll_delta = ui.ctx().input(|i| i.raw_scroll_delta.y);
    if !popup_open && scroll_delta != 0.0 && response.hovered() {
        let zoom_center = ui
            .ctx()
            .pointer_interact_pos()
            .unwrap_or(editor_rect.center());
        let local_center = zoom_center - editor_rect.min;
        let old_zoom = context.node_editor_state.target_zoom;
        let new_zoom = (old_zoom * (1.0 + scroll_delta * 0.002)).clamp(0.1, 5.0);
        let effective_zoom_delta = if old_zoom > 1e-6 {
            new_zoom / old_zoom
        } else {
            1.0
        };
        context.node_editor_state.target_zoom = new_zoom;
        let vec = context.node_editor_state.target_pan - local_center;
        context.node_editor_state.target_pan = local_center + vec * effective_zoom_delta;
        context.node_editor_state.zoom_center = Some(zoom_center);
        changed = true;
    }
    context.node_editor_state.target_zoom = context.node_editor_state.target_zoom.clamp(0.1, 5.0);

    context.node_editor_state.pan = context.node_editor_state.target_pan;
    context.node_editor_state.zoom = context.node_editor_state.target_zoom;
    if changed {
        context.ui_invalidator.request_repaint_after_tagged(
            "node_editor/pan_zoom",
            Duration::ZERO,
            RepaintCause::Input,
        );
    }
}

pub fn handle_connection_drop(
    editor: &mut NodeEditorTab,
    ui: &mut egui::Ui,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    let is_clicked = ui.input(|i| i.pointer.primary_clicked());
    let is_released = ui.input(|i| i.pointer.any_released());
    let is_esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let alt_down = ui.input(|i| i.modifiers.alt);

    // --- Multi-wire drag: only preview + (Alt on blank) => create Merge entity and auto-connect ---
    if !editor.pending_connections_from.is_empty() {
        if is_released || is_esc {
            editor.pending_connections_from.clear();
            editor.pending_connection_from = None;
            editor.snapped_to_port = None;
            editor.pending_wire_waypoints.clear();
            editor.pending_wire_parked = false;
            editor.single_alt_was_down = false;
            editor.selected_waypoint = None;
            editor.waypoint_drag_old = None;
            editor.multi_alt_was_down = false;
            return;
        }
        if alt_down && !editor.multi_alt_was_down && editor.snapped_to_port.is_none() {
            if let Some(p) = ui.ctx().pointer_interact_pos() {
                let pos = ((p - editor_rect.min - editor.pan) / editor.zoom).to_pos2();
                let merge = prepare_generic_node(
                    context.node_registry,
                    context.node_editor_settings,
                    pos,
                    "Merge",
                );
                let merge_id = merge.id;
                let to_port = merge
                    .inputs
                    .keys()
                    .next()
                    .cloned()
                    .unwrap_or_else(|| crate::nodes::PortId::from("Input"));
                let out_port = merge
                    .outputs
                    .keys()
                    .next()
                    .cloned()
                    .unwrap_or_else(|| crate::nodes::PortId::from("Output"));

                // Sort sources by left-to-right (Houdini-like).
                let mut srcs = editor.pending_connections_from.clone();
                srcs.sort_by(|(a, _), (b, _)| {
                    let ax = editor
                        .cached_nodes
                        .iter()
                        .find(|n| n.id == *a)
                        .map(|n| n.position.x)
                        .unwrap_or(0.0);
                    let bx = editor
                        .cached_nodes
                        .iter()
                        .find(|n| n.id == *b)
                        .map(|n| n.position.x)
                        .unwrap_or(0.0);
                    ax.partial_cmp(&bx).unwrap_or(std::cmp::Ordering::Equal)
                });

                let mut cmds: Vec<Box<dyn crate::cunning_core::command::Command>> = Vec::new();
                cmds.push(Box::new(CmdAddNode {
                    node: merge.clone(),
                }));
                for (i, (from_node, from_port)) in srcs.into_iter().enumerate() {
                    let conn = Connection {
                        id: ConnectionId::new_v4(),
                        from_node,
                        from_port,
                        to_node: merge_id,
                        to_port: to_port.clone(),
                        order: i as i32,
                        waypoints: Vec::new(),
                    };
                    cmds.push(Box::new(CmdSetConnection::new(conn, false)));
                }

                let root_graph = &mut context.node_graph_res.0;
                cda::navigation::with_current_graph_mut(root_graph, &editor.cda_state, |g| {
                    context
                        .node_editor_state
                        .execute(Box::new(CmdBatch::new("Multi Merge", cmds)), g);
                });
                context.graph_changed_writer.write_default();

                // Continue as a single wire from Merge output (still dragging).
                editor.pending_connections_from.clear();
                editor.pending_connection_from = Some((merge_id, out_port));
                editor.snapped_to_port = None;
                editor.did_start_connection_this_frame = true;
                editor.multi_alt_was_down = true;
            }
        }
        if !alt_down {
            editor.multi_alt_was_down = false;
        }
        return;
    }

    if editor.pending_connection_from.is_some()
        && !editor.did_start_connection_this_frame
        && (is_clicked || is_released || is_esc)
    {
        if let Some((start_node, start_port)) = &editor.pending_connection_from {
            if let Some((end_node, end_port)) = &editor.snapped_to_port {
                let mut root_graph = &mut context.node_graph_res.0;
                cda::navigation::with_current_graph_mut(
                    &mut root_graph,
                    &editor.cda_state,
                    |node_graph| {
                        let mut should_replace = true;
                        if let Some(target_node) = node_graph.nodes.get(end_node) {
                            if matches!(
                                target_node.input_style,
                                InputStyle::Bar | InputStyle::Collection
                            ) {
                                should_replace = false;
                            }
                            // If runtime policy is Error for this port, force replace to prevent "connect ok, but export/compile fails".
                            let reg = CdaRtRegistry::new_default();
                            if let Some(op) = reg.op_code_for_type(target_node.node_type.type_id())
                            {
                                if let Some(pol) = reg.in_port_policy_by_key(op, end_port) {
                                    if matches!(pol, MultiWirePolicy::Error) {
                                        should_replace = true;
                                    }
                                }
                            }
                        }
                        let new_conn_id = ConnectionId::new_v4();
                        let order = if should_replace {
                            0
                        } else {
                            node_graph
                                .connections
                                .values()
                                .filter(|c| c.to_node == *end_node && c.to_port == *end_port)
                                .map(|c| c.order)
                                .max()
                                .unwrap_or(-1)
                                + 1
                        };
                        let conn = Connection {
                            id: new_conn_id,
                            from_node: *start_node,
                            from_port: start_port.clone(),
                            to_node: *end_node,
                            to_port: end_port.clone(),
                            order,
                            waypoints: std::mem::take(&mut editor.pending_wire_waypoints),
                        };
                        editor.pending_wire_parked = false;
                        context.node_editor_state.execute(
                            Box::new(CmdSetConnection::new(conn, should_replace)),
                            node_graph,
                        );
                        context.graph_changed_writer.write_default();
                    },
                );
            }
            if editor.snapped_to_port.is_none() && is_released {
                if let Some(p) = ui.ctx().pointer_interact_pos() {
                    let anchor = (p - editor_rect.min - editor.pan) / editor.zoom;
                    editor.ghost_anchor_graph_pos = Some(anchor.to_pos2());
                }
            }
        }
        // Houdini-like "parked wire": if we released on blank canvas but already placed waypoints,
        // keep the dashed preview + relay dot so user can resume from it.
        if editor.snapped_to_port.is_none() && is_released && !editor.pending_wire_waypoints.is_empty() {
            editor.pending_wire_parked = true;
            editor.snapped_to_port = None;
            editor.did_start_connection_this_frame = false;
            return;
        }

        if (editor.snapped_to_port.is_none() && is_released)
            || editor.snapped_to_port.is_some()
            || is_esc
        {
            // Cancel any in-flight local AI requests
            if let Some(host) = context.native_ai_host {
                host.cancel_current();
            }
            if let Some(host) = context.tiny_model_host {
                host.cancel_current();
            }
            editor.pending_connection_from = None;
            editor.pending_wire_waypoints.clear();
            editor.pending_wire_parked = false;
            editor.single_alt_was_down = false;
            editor.selected_waypoint = None;
            editor.waypoint_drag_old = None;
            editor.snapped_to_port = None;
            editor.ghost_graph = None;
            editor.ghost_request_id = None;
            editor.ghost_anchor_graph_pos = None;
            editor.ghost_from = None;
            editor.ghost_multi_agg_name = None;
            editor.ghost_dialog.clear();
            editor.ghost_turns = 0;
            editor.ghost_pending_user = None;
            editor.ghost_reason_title = None;
            editor.ghost_reason = None;
            editor.ghost_params.clear();
            editor.deep_mode = false;
            editor.deep_skill_turns = 0;
            editor.deep_history.clear();
        }
    }
}

pub fn request_ghost_path(
    editor: &mut NodeEditorTab,
    ui: &egui::Ui,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    let backend = effective_backend(editor);
    editor.copilot_inflight_backend = Some(backend);
    editor.copilot_request_start = Some(std::time::Instant::now());
    let Some((start_node, start_port)) = editor.pending_connection_from.clone() else {
        return;
    };
    if editor.ghost_turns >= 15 {
        editor.ghost_dialog.clear();
        editor.ghost_turns = 0;
        editor.ghost_pending_user = None;
    }
    let anchor = if let Some(p) = editor.ghost_anchor_graph_pos {
        p
    } else if let Some(p) = ui.ctx().pointer_interact_pos() {
        let a = ((p - editor_rect.min - editor.pan) / editor.zoom).to_pos2();
        editor.ghost_anchor_graph_pos = Some(a);
        a
    } else {
        return;
    };
    editor.ghost_graph = None;
    editor.ghost_from = Some((start_node, start_port.clone()));
    editor.ghost_reason_title = None;
    editor.ghost_reason = None;
    let start_name = editor
        .cached_nodes
        .iter()
        .find(|n| n.id == start_node)
        .map(|n| n.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    let req_id = format!("ghost_path_{}", uuid::Uuid::new_v4());
    editor.ghost_request_id = Some(req_id.clone());

    let (_up_stats, up_full, up_sig, up_links) = {
        use crate::libs::ai_service::ai_defaults::{
            TINY_GHOST_MAX_FAR_NODES, TINY_GHOST_MAX_HOPS, TINY_GHOST_MAX_LINKS,
            TINY_GHOST_MAX_NODES,
        };
        let g = {
            let root = &context.node_graph_res.0;
            cda::navigation::graph_snapshot_by_path(&root, &editor.cda_state.breadcrumb())
        };
        let mut incoming: HashMap<
            crate::nodes::NodeId,
            Vec<(
                crate::nodes::NodeId,
                crate::nodes::PortId,
                crate::nodes::NodeId,
                crate::nodes::PortId,
            )>,
        > = HashMap::new();
        for c in g.connections.values() {
            incoming.entry(c.to_node).or_default().push((
                c.from_node,
                c.from_port,
                c.to_node,
                c.to_port,
            ));
        }
        let mut seen: HashSet<crate::nodes::NodeId> = HashSet::new();
        let mut q: VecDeque<(crate::nodes::NodeId, usize)> = VecDeque::new();
        let mut full_ids: Vec<crate::nodes::NodeId> = Vec::new();
        let mut sig_ids: Vec<crate::nodes::NodeId> = Vec::new();
        let mut links: Vec<(
            crate::nodes::NodeId,
            crate::nodes::PortId,
            crate::nodes::NodeId,
            crate::nodes::PortId,
        )> = Vec::new();
        q.push_back((start_node, 0));
        seen.insert(start_node);
        while let Some((nid, d)) = q.pop_front() {
            if nid != start_node {
                if full_ids.len() < TINY_GHOST_MAX_NODES {
                    full_ids.push(nid);
                } else if sig_ids.len() < TINY_GHOST_MAX_FAR_NODES {
                    sig_ids.push(nid);
                } else {
                    break;
                }
            }
            if d >= TINY_GHOST_MAX_HOPS {
                continue;
            }
            if let Some(incs) = incoming.get(&nid) {
                for (from, fp, to, tp) in incs {
                    if links.len() < TINY_GHOST_MAX_LINKS {
                        links.push((*from, fp.clone(), *to, tp.clone()));
                    }
                    if seen.insert(*from) {
                        q.push_back((*from, d + 1));
                    }
                }
            }
            if full_ids.len() >= TINY_GHOST_MAX_NODES
                && sig_ids.len() >= TINY_GHOST_MAX_FAR_NODES
                && links.len() >= TINY_GHOST_MAX_LINKS
            {
                break;
            }
        }
        let truncated = !q.is_empty()
            || sig_ids.len() >= TINY_GHOST_MAX_FAR_NODES
            || full_ids.len() >= TINY_GHOST_MAX_NODES
            || links.len() >= TINY_GHOST_MAX_LINKS;
        let kc =
            crate::libs::ai_service::native_tiny_model::knowledge_cache::GLOBAL_KNOWLEDGE_CACHE
                .get();
        let mut id_name: HashMap<crate::nodes::NodeId, String> = HashMap::new();
        for id in full_ids
            .iter()
            .chain(sig_ids.iter())
            .chain(links.iter().flat_map(|(a, _, b, _)| [a, b]))
        {
            let id = *id;
            id_name.entry(id).or_insert_with(|| {
                g.nodes
                    .get(&id)
                    .map(|n| n.name.clone())
                    .unwrap_or_else(|| id.to_string())
            });
        }
        let mut full = String::new();
        for (i, id) in full_ids.iter().enumerate() {
            let name = id_name.get(id).cloned().unwrap_or_else(|| id.to_string());
            let k = kc.and_then(|k| k.nodes.get(&name));
            let desc = k.map(|k| k.description.as_str()).unwrap_or("");
            let ins = k
                .map(|k| {
                    k.io.inputs
                        .iter()
                        .map(|p| format!("{}:{}", p.name, p.description))
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .unwrap_or_default();
            let outs = k
                .map(|k| {
                    k.io.outputs
                        .iter()
                        .map(|p| format!("{}:{}", p.name, p.description))
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .unwrap_or_default();
            full.push_str(&format!(
                "{}. {}\n- Description: {}\n- Inputs: {}\n- Outputs: {}\n",
                i + 1,
                name,
                desc,
                ins,
                outs
            ));
        }
        let mut sig = String::new();
        for id in &sig_ids {
            let name = id_name.get(id).cloned().unwrap_or_else(|| id.to_string());
            let k = kc.and_then(|k| k.nodes.get(&name));
            let input_type = k.map(|k| k.io.input_type.as_str()).unwrap_or("?");
            let ins = k
                .map(|k| {
                    k.io.inputs
                        .iter()
                        .map(|p| p.name.clone())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            sig.push_str(&format!(
                "- {}: input_type={}, inputs=[{}]\n",
                name, input_type, ins
            ));
        }
        let links_txt = links
            .iter()
            .enumerate()
            .map(|(i, (a, ap, b, bp))| {
                let an = id_name.get(a).cloned().unwrap_or_else(|| a.to_string());
                let bn = id_name.get(b).cloned().unwrap_or_else(|| b.to_string());
                format!("{}. {}::{} -> {}::{}", i + 1, an, ap, bn, bp)
            })
            .collect::<Vec<_>>()
            .join("\n");
        let stats = format!(
            "nodes_full={}/{}, nodes_sig={}/{}, links={}/{}, hops_cap={}, truncated={}",
            full_ids.len(),
            TINY_GHOST_MAX_NODES,
            sig_ids.len(),
            TINY_GHOST_MAX_FAR_NODES,
            links.len(),
            TINY_GHOST_MAX_LINKS,
            TINY_GHOST_MAX_HOPS,
            truncated
        );
        (stats, full, sig, links_txt)
    };

    let kc_ref = crate::libs::ai_service::native_tiny_model::knowledge_cache::GLOBAL_KNOWLEDGE_CACHE.get();
    let knowledge = kc_ref.and_then(|k| k.nodes.get(&start_name).cloned());
    // Build filtered knowledge based on start node category for reduced tokens
    let knowledge_filtered = if let Some(ref kc) = kc_ref {
        let start_cat = kc.get_category(&start_name).unwrap_or("Geometry");
        let related = crate::libs::ai_service::native_tiny_model::knowledge_cache::KnowledgeCache::related_categories(start_cat);
        kc.build_filtered_prompt(&related, Some(&start_name))
    } else {
        crate::libs::ai_service::native_tiny_model::knowledge_cache::GLOBAL_KNOWLEDGE_PROMPT_BLOB
            .get().cloned().unwrap_or_default()
    };
    let (desc, _out_ports) = if let Some(ref k) = knowledge {
        let outs = k.io.outputs.iter().map(|p| format!("{}:{}", p.name, p.description)).collect::<Vec<_>>().join("\n");
        (k.description.clone(), outs)
    } else {
        (String::new(), String::new())
    };
    let promote_hint = consume_selected_promote_notes(editor, context);
    let target_nodes = editor.ghost_target_nodes.take();
    let target_hint = target_nodes
        .map(|n| {
            format!(
                "\n\n[Goal]\nPlease generate about {} subsequent nodes (±2 allowed), not too many.",
                n
            )
        })
        .unwrap_or_default();

    let prompt = if editor.deep_mode {
        // Deep mode: use skill-based prompt, minimal knowledge injection
        use crate::libs::ai_service::copilot_skill::{build_deep_system_prompt, MAX_SKILL_TURNS};
        let sys = build_deep_system_prompt();
        let is_last_turn = editor.deep_skill_turns >= MAX_SKILL_TURNS - 1;
        let user_msg = if editor.deep_history.is_empty() {
            format!("{promote}\n[Context]\nStart node: {name}\nStart port: {port}\nDescription: {desc}{target}\n\nPlan node chain by calling skills, then output final JSON.", promote = promote_hint, name = start_name, port = start_port, desc = desc, target = target_hint)
        } else if is_last_turn {
            "This is your LAST turn. You MUST output final JSON now: {\"nodes\":[...], \"reason\":\"...\"}. No more skill calls allowed.".to_string()
        } else {
            "Continue planning. Call more skills or output final JSON.".to_string()
        };
        editor.ghost_pending_user = Some(user_msg.clone());
        format!("<|im_start|>system\n{sys}<|im_end|>\n{history}<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n<think>\n", sys = sys, history = editor.deep_history, user = user_msg)
    } else {
        // Fast mode: single-turn with filtered knowledge (reduced tokens)
        // Persist PromoteNote intent in dialog for multi-turn context
        if !promote_hint.is_empty() && editor.ghost_dialog.is_empty() {
            editor.ghost_dialog.push_str(&format!("<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\nUnderstood, I'll consider your intent.<|im_end|>\n", promote_hint.trim()));
        }
        let user_msg = if editor.ghost_dialog.is_empty() || !promote_hint.is_empty() {
            // Compact YAML-like format for context
            format!("{kb}\n\n[Context]\nstart: {name}\ndesc: {desc}\nport: {port}\nupstream: {up_count} nodes, {link_count} links\n{up_compact}{target}\n\nSuggest next nodes.", kb = knowledge_filtered, name = start_name, desc = desc, port = start_port, up_count = up_full.lines().count(), link_count = up_links.lines().count(), up_compact = if up_full.len() < 500 { format!("[Upstream]\n{}", up_full) } else { format!("[UpstreamSummary]\n{}", up_sig) }, target = target_hint)
        } else {
            format!("Wrong. Reconsider the connection intent and suggest better nodes.{target}\nOutput JSON only.", target = target_hint)
        };
        editor.ghost_pending_user = Some(user_msg.clone());
        format!("<|im_start|>system\nYou are Cunning3D Copilot. Return ONLY valid JSON.\nFormat: {{\"nodes\":[\"A\",\"B\"],\"params\":{{\"A.param\":\"val\"}},\"reason\":\"...\"}}\nRules: nodes from AllowedNodes only, params optional (non-numeric), reason<=3 lines\n<|im_end|>\n{dialog}<|im_start|>user\n{user}\n<|im_end|>\n<|im_start|>assistant\n", dialog = editor.ghost_dialog, user = user_msg)
    };
    // Deep mode forces local 4B Thinking; otherwise respect dropdown selection
    if editor.deep_mode {
        let Some(host) = context.native_ai_host else {
            bevy::log::warn!("DeepCopilot: native_ai_host (4B) is None");
            return;
        };
        host.request_prediction(req_id, prompt, 2048);
    } else {
        match backend {
            crate::tabs_system::node_editor::state::CopilotBackend::LocalTiny => {
                let Some(host) = context.tiny_model_host else {
                    return;
                };
                host.request(&req_id, &prompt);
            }
            crate::tabs_system::node_editor::state::CopilotBackend::LocalThink => {
                let Some(host) = context.native_ai_host else {
                    bevy::log::warn!("GhostCopilot: native_ai_host (4B) is None");
                    return;
                };
                host.request_prediction(req_id, prompt, 512);
            }
            crate::tabs_system::node_editor::state::CopilotBackend::Gemini => {
                let Some(host) = context.gemini_copilot_host else {
                    bevy::log::warn!(
                        "GhostCopilot: Gemini backend selected but gemini_copilot_host is None"
                    );
                    return;
                };
                host.request_with_model(&req_id, &prompt, Some(editor.gemini_cloud_model.model_name()));
            }
        }
    }
    let _ = anchor;
}

pub fn request_ghost_path_or_multi(
    editor: &mut NodeEditorTab,
    ui: &egui::Ui,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    if !editor.pending_connections_from.is_empty() {
        if let (Some(agg), Some(anchor)) = (
            editor.ghost_multi_agg_name.clone(),
            editor.ghost_anchor_graph_pos,
        ) {
            request_multi_path(editor, ui, context, editor_rect, anchor, &agg);
        } else {
            request_multi_agg_then_path(editor, ui, context, editor_rect);
        }
    } else {
        request_ghost_path(editor, ui, context, editor_rect);
    }
}

pub fn request_box_note(
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    if editor.box_note_request_id.is_some() {
        return;
    }
    let selected_nodes: Vec<_> = context.ui_state.selected_nodes.iter().copied().collect();
    let selected_stickies: Vec<_> = context
        .ui_state
        .selected_sticky_notes
        .iter()
        .copied()
        .collect();
    if selected_nodes.is_empty() && selected_stickies.is_empty() {
        return;
    }
    let req_id = format!("box_note_{}", uuid::Uuid::new_v4());
    editor.box_note_request_id = Some(req_id.clone());
    let backend = effective_backend(editor);
    editor.box_note_inflight_backend = Some(backend);
    editor.box_note_pending_nodes = selected_nodes.clone();
    editor.box_note_pending_stickies = selected_stickies.clone();

    let g = {
        let root = &context.node_graph_res.0;
        cda::navigation::graph_snapshot_by_path(&root, &editor.cda_state.breadcrumb())
    };
    let kc =
        crate::libs::ai_service::native_tiny_model::knowledge_cache::GLOBAL_KNOWLEDGE_CACHE.get();
    let mut nodes_txt = String::new();
    for (i, id) in selected_nodes.iter().enumerate() {
        let name = g
            .nodes
            .get(id)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| id.to_string());
        let desc = kc
            .and_then(|k| k.nodes.get(&name))
            .map(|k| k.description.clone())
            .unwrap_or_default();
        nodes_txt.push_str(&format!("{}. {} - {}\n", i + 1, name, desc));
    }
    let mut stickies_txt = String::new();
    for (i, id) in selected_stickies.iter().enumerate() {
        let content = g
            .sticky_notes
            .get(id)
            .map(|s| s.content.clone())
            .unwrap_or_default();
        if !content.is_empty() {
            stickies_txt.push_str(&format!("{}. {}\n", i + 1, content));
        }
    }
    let mut ext_inputs = Vec::new();
    for c in g.connections.values() {
        if selected_nodes.contains(&c.to_node) && !selected_nodes.contains(&c.from_node) {
            let from_name = g
                .nodes
                .get(&c.from_node)
                .map(|n| n.name.clone())
                .unwrap_or_else(|| c.from_node.to_string());
            let to_name = g
                .nodes
                .get(&c.to_node)
                .map(|n| n.name.clone())
                .unwrap_or_else(|| c.to_node.to_string());
            ext_inputs.push(format!(
                "{}::{} -> {}::{}",
                from_name, c.from_port, to_name, c.to_port
            ));
        }
    }
    let ext_txt = if ext_inputs.is_empty() {
        "None".to_string()
    } else {
        ext_inputs.join("\n")
    };

    let prompt = format!(
        "<|im_start|>system\nYou are Cunning3D annotation Copilot.\nReturn ONLY valid JSON object: {{\"title\":\"...\",\"content\":\"...\"}}.\nRules:\n- No extra text\n- title short (<=8 words)\n- content concise (<=6 lines)\n<|im_end|>\n<|im_start|>user\n[SelectedNodes]\n{nodes}\n\n[SelectedStickies]\n{stickies}\n\n[ExternalInputs]\n{ext}\n\nPlease summarize what this group does and write a clear annotation.\n<|im_end|>\n<|im_start|>assistant\n",
        nodes = nodes_txt, stickies = stickies_txt, ext = ext_txt
    );
    let _ = editor_rect;
    match backend {
        crate::tabs_system::node_editor::state::CopilotBackend::LocalTiny => {
            let Some(host) = context.tiny_model_host else {
                return;
            };
            host.request(&req_id, &prompt);
        }
        crate::tabs_system::node_editor::state::CopilotBackend::LocalThink => {
            let Some(host) = context.native_ai_host else {
                return;
            };
            host.request_prediction(req_id, prompt, 512);
        }
        crate::tabs_system::node_editor::state::CopilotBackend::Gemini => {
            let Some(host) = context.gemini_copilot_host else {
                bevy::log::warn!(
                    "BoxNote: Gemini backend selected but gemini_copilot_host is None"
                );
                return;
            };
            host.request_with_model(&req_id, &prompt, Some(editor.gemini_cloud_model.model_name()));
        }
    }
}

/// Request AI to explain selected nodes (triggered by ` key)
pub fn request_graph_explain(
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    screen_pos: Pos2,
) {
    if editor.explain_request_id.is_some() { return; }
    let selected: Vec<_> = context.ui_state.selected_nodes.iter().copied().collect();
    if selected.is_empty() { return; }
    let req_id = format!("explain_{}", uuid::Uuid::new_v4());
    editor.explain_request_id = Some(req_id.clone());
    let backend = effective_backend(editor);
    editor.explain_inflight_backend = Some(backend);
    editor.explain_pending_nodes = selected.clone();
    editor.explain_show_pos = Some(screen_pos);
    editor.explain_result = None;

    let g = {
        let root = &context.node_graph_res.0;
        crate::tabs_system::node_editor::cda::navigation::graph_snapshot_by_path(&root, &editor.cda_state.breadcrumb())
    };
    let kc = crate::libs::ai_service::native_tiny_model::knowledge_cache::GLOBAL_KNOWLEDGE_CACHE.get();
    // Build nodes list
    let mut nodes_txt = String::new();
    for (i, id) in selected.iter().enumerate() {
        let name = g.nodes.get(id).map(|n| n.name.clone()).unwrap_or_else(|| id.to_string());
        let desc = kc.and_then(|k| k.nodes.get(&name)).map(|k| k.description.as_str()).unwrap_or("");
        nodes_txt.push_str(&format!("{}. {} - {}\n", i + 1, name, desc));
    }
    // Build connections within selection
    let mut conns_txt = String::new();
    for c in g.connections.values() {
        if selected.contains(&c.from_node) && selected.contains(&c.to_node) {
            let from = g.nodes.get(&c.from_node).map(|n| n.name.as_str()).unwrap_or("?");
            let to = g.nodes.get(&c.to_node).map(|n| n.name.as_str()).unwrap_or("?");
            conns_txt.push_str(&format!("{}::{} -> {}::{}\n", from, c.from_port, to, c.to_port));
        }
    }
    // External inputs
    let mut ext_in = Vec::new();
    for c in g.connections.values() {
        if selected.contains(&c.to_node) && !selected.contains(&c.from_node) {
            let from = g.nodes.get(&c.from_node).map(|n| n.name.as_str()).unwrap_or("?");
            ext_in.push(format!("{}::{}", from, c.from_port));
        }
    }
    let prompt = format!(
        "<|im_start|>system\nYou are Cunning3D assistant. Explain what this node network does.\nReturn JSON: {{\"title\":\"short title\",\"explanation\":\"2-3 sentences\"}}\n<|im_end|>\n<|im_start|>user\n[Nodes]\n{nodes}\n[InternalConnections]\n{conns}\n[ExternalInputs]\n{ext}\n\nExplain this network.\n<|im_end|>\n<|im_start|>assistant\n",
        nodes = nodes_txt, conns = if conns_txt.is_empty() { "None".to_string() } else { conns_txt }, ext = if ext_in.is_empty() { "None".to_string() } else { ext_in.join(", ") }
    );
    match backend {
        crate::tabs_system::node_editor::state::CopilotBackend::LocalTiny => {
            if let Some(host) = context.tiny_model_host { host.request(&req_id, &prompt); }
        }
        crate::tabs_system::node_editor::state::CopilotBackend::LocalThink => {
            if let Some(host) = context.native_ai_host { host.request_prediction(req_id, prompt, 256); }
        }
        crate::tabs_system::node_editor::state::CopilotBackend::Gemini => {
            if let Some(host) = context.gemini_copilot_host { host.request_with_model(&req_id, &prompt, Some(editor.gemini_cloud_model.model_name())); }
        }
    }
}

/// Generate bindings_json for a Coverlay Panel node (Gemini only).
pub fn request_coverlay_panel_generate(editor: &mut NodeEditorTab, context: &mut EditorTabContext) {
    if editor.coverlay_gen_request_id.is_some() {
        return;
    }
    let selected: Vec<_> = context.ui_state.selected_nodes.iter().copied().collect();
    if selected.is_empty() {
        editor.coverlay_gen_error = Some("Select nodes first.".to_string());
        return;
    }
    let Some(host) = context.gemini_copilot_host else {
        editor.coverlay_gen_error = Some("GeminiCopilotHost is None.".to_string());
        return;
    };
    let req_id = format!("coverlay_panel_{}", uuid::Uuid::new_v4());
    editor.coverlay_gen_request_id = Some(req_id.clone());
    editor.coverlay_gen_error = None;

    let root = &context.node_graph_res.0;
    let g = crate::tabs_system::node_editor::cda::navigation::graph_snapshot_by_path(
        &root,
        &editor.cda_state.breadcrumb(),
    );
    let pv_kind = |v: &crate::nodes::parameter::ParameterValue| -> &'static str {
        use crate::nodes::parameter::ParameterValue::*;
        match v {
            Float(_) => "Float",
            Int(_) => "Int",
            Vec2(_) => "Vec2",
            Vec3(_) => "Vec3",
            Vec4(_) => "Vec4",
            IVec2(_) => "IVec2",
            String(_) => "String",
            Color(_) => "Color",
            Color4(_) => "Color4",
            Bool(_) => "Bool",
            Curve(_) => "Curve",
            UnitySpline(_) => "UnitySpline",
            Volume(_) => "Volume",
        }
    };
    let mut nodes_txt = String::new();
    for (i, id) in selected.iter().enumerate() {
        let Some(n) = g.nodes.get(id) else { continue; };
        nodes_txt.push_str(&format!("{}. id={} name={}\n", i + 1, id, n.name));
        for p in &n.parameters {
            nodes_txt.push_str(&format!(
                "   - {} ({}) ui={:?} value={}\n",
                p.name,
                p.label,
                p.ui_type,
                pv_kind(&p.value)
            ));
        }
    }
    let user = editor.coverlay_gen_prompt.trim();
    let prompt = format!(
        "<|im_start|>system\nYou generate Cunning3D Coverlay Panel bindings.\nReturn ONLY strict JSON with shape:\n{{\"title\":\"...\",\"bindings\":[{{\"target_node\":\"<uuid>\",\"target_param\":\"param_name\",\"label\":\"...\",\"ui\":{{\"kind\":\"auto\"}}}}]}}\nRules:\n- target_node MUST be one of the ids in [Nodes]\n- bindings <= 12\n- Prefer Float/Int/Bool/Color/Vec2/Vec3/Vec4/Dropdown-like params\n<|im_end|>\n<|im_start|>user\n[Instruction]\n{user}\n\n[Nodes]\n{nodes}\n<|im_end|>\n<|im_start|>assistant\n",
        user = user,
        nodes = nodes_txt
    );
    host.request_with_model(&req_id, &prompt, Some(editor.gemini_cloud_model.model_name()));
}

fn request_multi_agg_then_path(
    editor: &mut NodeEditorTab,
    ui: &egui::Ui,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    editor.copilot_inflight_backend = Some(editor.copilot_backend);
    let n = editor.pending_connections_from.len();
    let anchor = if let Some(p) = editor.ghost_anchor_graph_pos {
        p
    } else if let Some(p) = ui.ctx().pointer_interact_pos() {
        let a = ((p - editor_rect.min - editor.pan) / editor.zoom).to_pos2();
        editor.ghost_anchor_graph_pos = Some(a);
        a
    } else {
        return;
    };
    editor.ghost_graph = None;
    editor.ghost_from = None;
    editor.ghost_multi_agg_name = None;

    // N>=6: fixed Merge.
    if n >= 6 {
        editor.ghost_multi_agg_name = Some("Merge".to_string());
        request_multi_path(editor, ui, context, editor_rect, anchor, "Merge");
        return;
    }

    // N=2..5: ask tiny model to pick an aggregator from filtered candidates.
    let kc =
        crate::libs::ai_service::native_tiny_model::knowledge_cache::GLOBAL_KNOWLEDGE_CACHE.get();
    let nodes = kc.map(|k| k.nodes.clone());
    let mut candidates: Vec<(String, i32, usize, String)> = Vec::new(); // (name, score, inputs, input_type)
    if let Some(map) = nodes.as_ref() {
        for (name, k) in map.iter() {
            let ins = k.io.inputs.len();
            let is_multi = k.io.input_type.eq_ignore_ascii_case("Multi");
            if ins >= n || is_multi {
                let text = format!("{} {} {}", k.category, k.description, name).to_lowercase();
                let mut score = 0;
                if text.contains("merge") || text.contains("合并") || text.contains("汇总") {
                    score += 50;
                }
                if text.contains("boolean") || text.contains("布尔") {
                    score += 30;
                }
                if text.contains("switch") || text.contains("开关") {
                    score += 20;
                }
                if text.contains("combine") || text.contains("组合") {
                    score += 20;
                }
                if ins >= n {
                    score += 10;
                }
                if is_multi {
                    score += 5;
                }
                candidates.push((name.clone(), score, ins, k.io.input_type.clone()));
            }
        }
    }
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let top = candidates.into_iter().take(48).collect::<Vec<_>>();
    if top.is_empty() {
        editor.ghost_multi_agg_name = Some("Merge".to_string());
        request_multi_path(editor, ui, context, editor_rect, anchor, "Merge");
        return;
    }

    let sources_txt = editor
        .pending_connections_from
        .iter()
        .map(|(nid, pid)| {
            let name = editor
                .cached_nodes
                .iter()
                .find(|n| n.id == *nid)
                .map(|n| n.name.clone())
                .unwrap_or_else(|| nid.to_string());
            format!("- {}::{}", name, pid)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let cand_txt = top
        .iter()
        .map(|(name, _score, ins, it)| format!("- {} | inputs={} | input_type={}", name, ins, it))
        .collect::<Vec<_>>()
        .join("\n");
    let req_id = format!("ghost_agg_{}", uuid::Uuid::new_v4());
    editor.ghost_request_id = Some(req_id.clone());
    editor.ghost_pending_user = Some(format!("[Task] N-way input aggregator selection\nN={}\n\n[Sources]\n{}\n\n[CandidateAllowedNodes]\n{}\n\nOutput only a JSON array containing exactly 1 node name. Example: [\"Merge\"]", n, sources_txt, cand_txt));
    let prompt = format!(
        "<|im_start|>system\nYou are Cunning3D Node Graph Copilot: Aggregator Selector.\nScenario: User drags N outputs simultaneously and needs to select an 'aggregator node' to merge N paths into 1.\nRequirements:\n- No explanation, no thinking process output\n- Output only a JSON array containing exactly 1 node name, e.g.: [\"Merge\"]\n- Node name must be from CandidateAllowedNodes\n<|im_end|>\n{dialog}<|im_start|>user\n{user}\n<|im_end|>\n<|im_start|>assistant\n",
        dialog = editor.ghost_dialog,
        user = editor.ghost_pending_user.clone().unwrap_or_default()
    );
    match editor.copilot_backend {
        crate::tabs_system::node_editor::state::CopilotBackend::LocalTiny => {
            let Some(host) = context.tiny_model_host else {
                return;
            };
            host.request(&req_id, &prompt);
        }
        crate::tabs_system::node_editor::state::CopilotBackend::LocalThink => {
            let Some(host) = context.native_ai_host else {
                return;
            };
            host.request_prediction(req_id, prompt, 256);
        }
        crate::tabs_system::node_editor::state::CopilotBackend::Gemini => {
            let Some(host) = context.gemini_copilot_host else {
                bevy::log::warn!(
                    "GhostCopilot: Gemini backend selected but gemini_copilot_host is None"
                );
                return;
            };
            host.request_with_model(&req_id, &prompt, Some(editor.gemini_cloud_model.model_name()));
        }
    }
    let _ = anchor;
}

fn request_multi_path(
    editor: &mut NodeEditorTab,
    _ui: &egui::Ui,
    context: &mut EditorTabContext,
    editor_rect: Rect,
    anchor: Pos2,
    agg_name: &str,
) {
    editor.copilot_inflight_backend = Some(editor.copilot_backend);
    let kc = crate::libs::ai_service::native_tiny_model::knowledge_cache::GLOBAL_KNOWLEDGE_CACHE.get();
    // Build filtered knowledge based on aggregator category
    let knowledge_filtered = if let Some(ref cache) = kc {
        let agg_cat = cache.get_category(agg_name).unwrap_or("Geometry");
        let related = crate::libs::ai_service::native_tiny_model::knowledge_cache::KnowledgeCache::related_categories(agg_cat);
        cache.build_filtered_prompt(&related, Some(agg_name))
    } else {
        crate::libs::ai_service::native_tiny_model::knowledge_cache::GLOBAL_KNOWLEDGE_PROMPT_BLOB.get().cloned().unwrap_or_default()
    };
    let agg_desc = kc
        .and_then(|k| k.nodes.get(agg_name))
        .map(|k| k.description.as_str())
        .unwrap_or("");
    let out_port = kc
        .and_then(|k| k.nodes.get(agg_name))
        .and_then(|k| k.io.outputs.first())
        .map(|p| p.name.as_str())
        .unwrap_or("Output");
    let target_nodes = editor.ghost_target_nodes.take();
    let target_hint = target_nodes
        .map(|n| {
            format!(
                "\n\n[Goal]\nPlease generate about {} subsequent nodes (±2 allowed), not too many.",
                n
            )
        })
        .unwrap_or_default();

    let sources_txt = editor
        .pending_connections_from
        .iter()
        .map(|(nid, pid)| {
            let name = editor
                .cached_nodes
                .iter()
                .find(|n| n.id == *nid)
                .map(|n| n.name.clone())
                .unwrap_or_else(|| nid.to_string());
            let desc = kc
                .and_then(|k| k.nodes.get(&name))
                .map(|k| k.description.as_str())
                .unwrap_or("");
            format!("- {}::{} | {}", name, pid, desc)
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Upstream context: BFS union from all sources with existing budgets.
    let (_up_stats, up_full, up_sig, _up_links) = {
        use crate::libs::ai_service::ai_defaults::{
            TINY_GHOST_MAX_FAR_NODES, TINY_GHOST_MAX_HOPS, TINY_GHOST_MAX_LINKS,
            TINY_GHOST_MAX_NODES,
        };
        let g = {
            let root = &context.node_graph_res.0;
            cda::navigation::graph_snapshot_by_path(&root, &editor.cda_state.breadcrumb())
        };
        let mut incoming: HashMap<
            crate::nodes::NodeId,
            Vec<(
                crate::nodes::NodeId,
                crate::nodes::PortId,
                crate::nodes::NodeId,
                crate::nodes::PortId,
            )>,
        > = HashMap::new();
        for c in g.connections.values() {
            incoming.entry(c.to_node).or_default().push((
                c.from_node,
                c.from_port,
                c.to_node,
                c.to_port,
            ));
        }
        let mut seen: HashSet<crate::nodes::NodeId> = HashSet::new();
        let mut q: VecDeque<(crate::nodes::NodeId, usize)> = VecDeque::new();
        for (nid, _) in editor.pending_connections_from.iter().cloned() {
            if seen.insert(nid) {
                q.push_back((nid, 0));
            }
        }
        let mut full_ids: Vec<crate::nodes::NodeId> = Vec::new();
        let mut sig_ids: Vec<crate::nodes::NodeId> = Vec::new();
        let mut links: Vec<(
            crate::nodes::NodeId,
            crate::nodes::PortId,
            crate::nodes::NodeId,
            crate::nodes::PortId,
        )> = Vec::new();
        while let Some((nid, d)) = q.pop_front() {
            if d > 0 {
                if full_ids.len() < TINY_GHOST_MAX_NODES {
                    full_ids.push(nid);
                } else if sig_ids.len() < TINY_GHOST_MAX_FAR_NODES {
                    sig_ids.push(nid);
                } else {
                    break;
                }
            }
            if d >= TINY_GHOST_MAX_HOPS {
                continue;
            }
            if let Some(incs) = incoming.get(&nid) {
                for (from, fp, to, tp) in incs {
                    if links.len() < TINY_GHOST_MAX_LINKS {
                        links.push((*from, *fp, *to, *tp));
                    }
                    if seen.insert(*from) {
                        q.push_back((*from, d + 1));
                    }
                }
            }
            if full_ids.len() >= TINY_GHOST_MAX_NODES
                && sig_ids.len() >= TINY_GHOST_MAX_FAR_NODES
                && links.len() >= TINY_GHOST_MAX_LINKS
            {
                break;
            }
        }
        let truncated = !q.is_empty()
            || sig_ids.len() >= TINY_GHOST_MAX_FAR_NODES
            || full_ids.len() >= TINY_GHOST_MAX_NODES
            || links.len() >= TINY_GHOST_MAX_LINKS;
        let kc =
            crate::libs::ai_service::native_tiny_model::knowledge_cache::GLOBAL_KNOWLEDGE_CACHE
                .get();
        let mut id_name: HashMap<crate::nodes::NodeId, String> = HashMap::new();
        for id in full_ids
            .iter()
            .chain(sig_ids.iter())
            .chain(links.iter().flat_map(|(a, _, b, _)| [a, b]))
        {
            let id = *id;
            id_name.entry(id).or_insert_with(|| {
                g.nodes
                    .get(&id)
                    .map(|n| n.name.clone())
                    .unwrap_or_else(|| id.to_string())
            });
        }
        let mut full = String::new();
        for (i, id) in full_ids.iter().enumerate() {
            let name = id_name.get(id).cloned().unwrap_or_else(|| id.to_string());
            let k = kc.and_then(|k| k.nodes.get(&name));
            let desc = k.map(|k| k.description.as_str()).unwrap_or("");
            let ins = k
                .map(|k| {
                    k.io.inputs
                        .iter()
                        .map(|p| format!("{}:{}", p.name, p.description))
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .unwrap_or_default();
            let outs = k
                .map(|k| {
                    k.io.outputs
                        .iter()
                        .map(|p| format!("{}:{}", p.name, p.description))
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .unwrap_or_default();
            full.push_str(&format!(
                "{}. {}\n- Description: {}\n- Inputs: {}\n- Outputs: {}\n",
                i + 1,
                name,
                desc,
                ins,
                outs
            ));
        }
        let mut sig = String::new();
        for id in &sig_ids {
            let name = id_name.get(id).cloned().unwrap_or_else(|| id.to_string());
            let k = kc.and_then(|k| k.nodes.get(&name));
            let input_type = k.map(|k| k.io.input_type.as_str()).unwrap_or("?");
            let ins = k
                .map(|k| {
                    k.io.inputs
                        .iter()
                        .map(|p| p.name.clone())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            sig.push_str(&format!(
                "- {}: input_type={}, inputs=[{}]\n",
                name, input_type, ins
            ));
        }
        let links_txt = links
            .iter()
            .enumerate()
            .map(|(i, (a, ap, b, bp))| {
                let an = id_name.get(a).cloned().unwrap_or_else(|| a.to_string());
                let bn = id_name.get(b).cloned().unwrap_or_else(|| b.to_string());
                format!("{}. {}::{} -> {}::{}", i + 1, an, ap, bn, bp)
            })
            .collect::<Vec<_>>()
            .join("\n");
        let stats = format!(
            "nodes_full={}/{}, nodes_sig={}/{}, links={}/{}, hops_cap={}, truncated={}",
            full_ids.len(),
            TINY_GHOST_MAX_NODES,
            sig_ids.len(),
            TINY_GHOST_MAX_FAR_NODES,
            links.len(),
            TINY_GHOST_MAX_LINKS,
            TINY_GHOST_MAX_HOPS,
            truncated
        );
        (stats, full, sig, links_txt)
    };

    let req_id = format!("ghost_pathmw_{}_{}", agg_name, uuid::Uuid::new_v4());
    editor.ghost_request_id = Some(req_id.clone());
    editor.ghost_reason_title = None;
    editor.ghost_reason = None;
    // Compact multi-wire context
    let up_compact = if up_full.len() < 400 { up_full.clone() } else { up_sig.clone() };
    editor.ghost_pending_user = Some(format!(
        "{kb}\n\n[MultiWire]\nN={n}\nAgg={agg}\nsources:\n{sources}\nupstream: {up_count} nodes\n{up}\nstart: {agg}|{desc}|port={port}{target}\n\nSuggest next nodes.",
        kb = knowledge_filtered,
        n = editor.pending_connections_from.len(),
        agg = agg_name,
        sources = sources_txt,
        up_count = up_full.lines().count(),
        up = up_compact,
        desc = agg_desc,
        port = out_port,
        target = target_hint
    ));
    let prompt = format!(
        "<|im_start|>system\nYou are Cunning3D Copilot. Return ONLY valid JSON.\nFormat: {{\"nodes\":[\"A\",\"B\"],\"params\":{{\"A.param\":\"val\"}},\"reason\":\"...\"}}\nRules: nodes from AllowedNodes only, params optional (non-numeric), reason<=3 lines\n<|im_end|>\n{dialog}<|im_start|>user\n{user}\n<|im_end|>\n<|im_start|>assistant\n",
        dialog = editor.ghost_dialog,
        user = editor.ghost_pending_user.clone().unwrap_or_default()
    );
    let _ = (anchor, editor_rect);
    match editor.copilot_backend {
        crate::tabs_system::node_editor::state::CopilotBackend::LocalTiny => {
            let Some(host) = context.tiny_model_host else {
                return;
            };
            host.request(&req_id, &prompt);
        }
        crate::tabs_system::node_editor::state::CopilotBackend::LocalThink => {
            let Some(host) = context.native_ai_host else {
                return;
            };
            host.request_prediction(req_id, prompt, 512);
        }
        crate::tabs_system::node_editor::state::CopilotBackend::Gemini => {
            let Some(host) = context.gemini_copilot_host else {
                bevy::log::warn!(
                    "GhostCopilot: Gemini backend selected but gemini_copilot_host is None"
                );
                return;
            };
            host.request_with_model(&req_id, &prompt, Some(editor.gemini_cloud_model.model_name()));
        }
    }
}

pub fn handle_deferred_actions(
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    if editor.create_network_box_request
        || editor.create_sticky_note_request
        || editor.create_promote_note_request
    {
        handle_top_bar_actions(editor, context, editor_rect);
    }
}

pub(crate) fn auto_place_note_rect(g: &crate::nodes::structs::NodeGraph, desired: Rect) -> Rect {
    let pad = 18.0;
    let blocked = |r: Rect| -> bool {
        for n in g.nodes.values() {
            if Rect::from_min_size(n.position, n.size).expand(pad).intersects(r) {
                return true;
            }
        }
        for s in g.sticky_notes.values() {
            if s.rect.expand(pad).intersects(r) {
                return true;
            }
        }
        for b in g.network_boxes.values() {
            if b.rect.expand(pad).intersects(r) {
                return true;
            }
        }
        false
    };
    if !blocked(desired) {
        return desired;
    }
    let step = 56.0;
    let c0 = desired.center();
    let sz = desired.size();
    for r in 1i32..=14 {
        for ox in -r..=r {
            for oy in -r..=r {
                if ox.abs() != r && oy.abs() != r {
                    continue;
                }
                let c = c0 + Vec2::new(ox as f32 * step, oy as f32 * step);
                let cand = Rect::from_center_size(c, sz);
                if !blocked(cand) {
                    return cand;
                }
            }
        }
    }
    desired
}

pub fn handle_top_bar_actions(
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    if editor.create_sticky_note_request {
        let root_graph = &mut context.node_graph_res.0;
        cda::navigation::with_current_graph_mut(root_graph, &editor.cda_state, |node_graph| {
            let new_note_id = StickyNoteId::new_v4();

            let view_center_graph_pos =
                (editor_rect.center() - editor_rect.min - editor.pan) / editor.zoom;
            let new_note_size = Vec2::new(200.0, 150.0);
            let new_note_rect = auto_place_note_rect(
                node_graph,
                Rect::from_center_size(view_center_graph_pos.to_pos2(), new_note_size),
            );

            let new_note = StickyNote {
                id: new_note_id,
                rect: new_note_rect,
                title: "Sticky Note".to_string(),
                content: String::new(),
                color: Color32::from_rgb(255, 243, 138),
            };

            context
                .node_editor_state
                .execute(Box::new(CmdAddStickyNote { note: new_note }), node_graph);
            context.graph_changed_writer.write_default();

            editor.create_sticky_note_request = false;
        });
    }

    if editor.create_network_box_request {
        let root_graph = &mut context.node_graph_res.0;
        cda::navigation::with_current_graph_mut(root_graph, &editor.cda_state, |node_graph| {
            let new_box_id = NetworkBoxId::new_v4();

            let (new_box_rect, nodes_to_include) = if context.ui_state.selected_nodes.is_empty() {
                let view_center_graph_pos =
                    (editor_rect.center() - editor_rect.min - editor.pan) / editor.zoom;
                let new_box_size = Vec2::new(300.0, 200.0);
                (
                    Rect::from_center_size(view_center_graph_pos.to_pos2(), new_box_size),
                    Default::default(),
                )
            } else {
                let mut min_pos = Pos2::new(f32::MAX, f32::MAX);
                let mut max_pos = Pos2::new(f32::MIN, f32::MIN);

                for node_id in &context.ui_state.selected_nodes {
                    if let Some(node) = node_graph.nodes.get(node_id) {
                        min_pos.x = min_pos.x.min(node.position.x);
                        min_pos.y = min_pos.y.min(node.position.y);
                        max_pos.x = max_pos.x.max(node.position.x + node.size.x);
                        max_pos.y = max_pos.y.max(node.position.y + node.size.y);
                    }
                }

                let padding = Vec2::new(40.0, 40.0);
                (
                    Rect::from_min_max(min_pos - padding, max_pos + padding),
                    context.ui_state.selected_nodes.clone(),
                )
            };

            let new_box = NetworkBox {
                id: new_box_id,
                rect: new_box_rect,
                title: "Network Box".to_string(),
                color: Color32::from_rgba_unmultiplied(50, 50, 80, 100),
                nodes_inside: nodes_to_include,
                stickies_inside: HashSet::new(),
            };

            context
                .node_editor_state
                .execute(Box::new(CmdAddNetworkBox { box_: new_box }), node_graph);
            context.graph_changed_writer.write_default();

            editor.create_network_box_request = false;
        });
    }

    if editor.create_promote_note_request {
        let view_center_graph_pos =
            (editor_rect.center() - editor_rect.min - editor.pan) / editor.zoom;
        let new_note = promote_note::create_promote_note(view_center_graph_pos.to_pos2());
        let root_graph = &mut context.node_graph_res.0;
        cda::navigation::with_current_graph_mut(root_graph, &editor.cda_state, |g| {
            context.node_editor_state.execute(
                Box::new(crate::cunning_core::command::basic::CmdAddPromoteNote { note: new_note }),
                g,
            );
            context.graph_changed_writer.write_default();
        });
        editor.create_promote_note_request = false;
    }
}

pub fn update_node_animations(
    editor: &mut NodeEditorTab,
    ui: &mut egui::Ui,
    context: &mut EditorTabContext,
) {
    let current_time = ui.ctx().input(|i| i.time);
    let mut finished = Vec::new();
    let mut graph_needs_update = false;
    let mut any_active = false;
    {
        let root_graph = &mut context.node_graph_res.0;
        cda::navigation::with_current_graph_mut(root_graph, &editor.cda_state, |node_graph| {
            for (node_id, anim) in &mut editor.node_animations {
                let elapsed = current_time - anim.start_time;
                let t = (elapsed / anim.duration) as f32;
                if t >= 1.0 {
                    if let Some(node) = node_graph.nodes.get_mut(&node_id) {
                        node.position = anim.target_pos;
                    }
                    finished.push(*node_id);
                } else {
                    let t = 1.0 - (1.0 - t).powi(3);
                    if let Some(node) = node_graph.nodes.get_mut(&node_id) {
                        node.position = anim.start_pos.lerp(anim.target_pos, t);
                    }
                    any_active = true;
                }
                graph_needs_update = true;
            }
        });
    }
    if any_active {
        context.ui_invalidator.request_repaint_after_tagged(
            "node_editor/node_anim",
            Duration::from_secs_f32(1.0 / 60.0),
            RepaintCause::Animation,
        );
    }
    if graph_needs_update {
        context.graph_changed_writer.write_default();
    }
    for id in finished {
        editor.node_animations.remove(&id);
    }
}
