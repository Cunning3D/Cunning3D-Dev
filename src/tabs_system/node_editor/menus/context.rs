#![allow(deprecated)]
use crate::cunning_core::command::basic::CmdDeleteNodes;
use crate::cunning_core::command::basic::{
    CmdAddNetworkBox, CmdAddNode, CmdBatch, CmdSetConnection,
};
use crate::cunning_core::command::basic::{CmdReplaceGraph, GraphSnapshot};
use crate::invalidator::RepaintCause;
use crate::nodes::flow::spawn::{
    build_foreach_block, build_foreach_connectivity_block, build_foreach_point_block,
    build_foreach_primitive_block,
};
use crate::{
    nodes::structs::{CDANodeData, Node, NodeId, NodeType},
    tabs_system::{
        node_editor::{cda, state::MenuState, NodeEditorTab},
        EditorTabContext,
    },
    ui::prepare_generic_node,
};
use bevy_egui::egui::{self, Id, Pos2, Rect};
use rfd::FileDialog;
use std::time::Duration;

fn hit_node_at(editor: &NodeEditorTab, screen_pos: Pos2, editor_rect: Rect) -> Option<NodeId> {
    let gp = ((screen_pos - editor_rect.min - editor.pan) / editor.zoom).to_pos2();
    let bs = editor.hit_cache.bucket_size.max(1.0);
    let key = ((gp.x / bs).floor() as i32, (gp.y / bs).floor() as i32);
    editor.hit_cache.buckets.get(&key).and_then(|v| {
        v.iter().rev().find_map(|&i| {
            editor.hit_cache.nodes.get(i).and_then(|n| {
                if n.rect.contains(gp) {
                    Some(n.id)
                } else {
                    None
                }
            })
        })
    })
}

fn close_menu(ctx: &egui::Context, editor: &mut NodeEditorTab, surrender_focus: Option<Id>) {
    editor.menu_state = MenuState::None;
    editor.menu_search_text.clear();
    editor.menu_search_cached_query.clear();
    editor.menu_search_cached_results.clear();
    editor.menu_search_cached_categories.clear();
    if let Some(id) = surrender_focus {
        ctx.memory_mut(|m| m.surrender_focus(id));
    }
}

fn spawn_foreach_block(
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    spawn: egui::Pos2,
    focus_end: bool,
) {
    let root = &mut context.node_graph_res.0;
    cda::navigation::with_current_graph_mut(root, &editor.cda_state, |g| {
        let spec = build_foreach_block(
            g,
            context.node_registry,
            Some(context.node_editor_settings),
            spawn,
            false,
        );
        let created: Vec<crate::nodes::NodeId> = spec.nodes.iter().map(|n| n.id).collect();
        let bid = spec.network_box.id;
        let mut cmds: Vec<Box<dyn crate::cunning_core::command::Command>> = Vec::new();
        for n in spec.nodes {
            cmds.push(Box::new(CmdAddNode { node: n }));
        }
        for c in spec.connections {
            cmds.push(Box::new(CmdSetConnection::new(c, true)));
        }
        cmds.push(Box::new(CmdAddNetworkBox {
            box_: spec.network_box,
        }));
        context
            .node_editor_state
            .execute(Box::new(CmdBatch::new("Create ForEach Block", cmds)), g);
        context.ui_state.selected_nodes.clear();
        for id in &created {
            context.ui_state.selected_nodes.insert(*id);
        }
        context.ui_state.last_selected_node_id = if focus_end {
            created.get(1).copied().or(created.first().copied())
        } else {
            created.first().copied()
        };
        context.ui_state.selected_network_boxes.clear();
        context.ui_state.selected_network_boxes.insert(bid);
    });
}

fn spawn_foreach_connectivity_block(
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    spawn: egui::Pos2,
) {
    let root = &mut context.node_graph_res.0;
    cda::navigation::with_current_graph_mut(root, &editor.cda_state, |g| {
        let spec = build_foreach_connectivity_block(
            g,
            context.node_registry,
            Some(context.node_editor_settings),
            spawn,
            false,
        );
        let created: Vec<crate::nodes::NodeId> = spec.nodes.iter().map(|n| n.id).collect();
        let bid = spec.network_box.id;
        let mut cmds: Vec<Box<dyn crate::cunning_core::command::Command>> = Vec::new();
        for n in spec.nodes {
            cmds.push(Box::new(CmdAddNode { node: n }));
        }
        for c in spec.connections {
            cmds.push(Box::new(CmdSetConnection::new(c, true)));
        }
        cmds.push(Box::new(CmdAddNetworkBox {
            box_: spec.network_box,
        }));
        context.node_editor_state.execute(
            Box::new(CmdBatch::new("Create ForEach Connectivity Block", cmds)),
            g,
        );
        context.ui_state.selected_nodes.clear();
        for id in &created {
            context.ui_state.selected_nodes.insert(*id);
        }
        context.ui_state.last_selected_node_id = created
            .get(2)
            .copied()
            .or(created.last().copied())
            .or(created.first().copied());
        context.ui_state.selected_network_boxes.clear();
        context.ui_state.selected_network_boxes.insert(bid);
    });
}

fn spawn_foreach_point_block(
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    spawn: egui::Pos2,
) {
    let root = &mut context.node_graph_res.0;
    cda::navigation::with_current_graph_mut(root, &editor.cda_state, |g| {
        let spec = build_foreach_point_block(
            g,
            context.node_registry,
            Some(context.node_editor_settings),
            spawn,
            false,
        );
        let created: Vec<crate::nodes::NodeId> = spec.nodes.iter().map(|n| n.id).collect();
        let bid = spec.network_box.id;
        let mut cmds: Vec<Box<dyn crate::cunning_core::command::Command>> = Vec::new();
        for n in spec.nodes {
            cmds.push(Box::new(CmdAddNode { node: n }));
        }
        for c in spec.connections {
            cmds.push(Box::new(CmdSetConnection::new(c, true)));
        }
        cmds.push(Box::new(CmdAddNetworkBox {
            box_: spec.network_box,
        }));
        context.node_editor_state.execute(
            Box::new(CmdBatch::new("Create ForEach Point Block", cmds)),
            g,
        );
        context.ui_state.selected_nodes.clear();
        for id in &created {
            context.ui_state.selected_nodes.insert(*id);
        }
        context.ui_state.last_selected_node_id = created
            .get(1)
            .copied()
            .or(created.last().copied())
            .or(created.first().copied());
        context.ui_state.selected_network_boxes.clear();
        context.ui_state.selected_network_boxes.insert(bid);
    });
}

fn spawn_foreach_primitive_block(
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    spawn: egui::Pos2,
) {
    let root = &mut context.node_graph_res.0;
    cda::navigation::with_current_graph_mut(root, &editor.cda_state, |g| {
        let spec = build_foreach_primitive_block(
            g,
            context.node_registry,
            Some(context.node_editor_settings),
            spawn,
            false,
        );
        let created: Vec<crate::nodes::NodeId> = spec.nodes.iter().map(|n| n.id).collect();
        let bid = spec.network_box.id;
        let mut cmds: Vec<Box<dyn crate::cunning_core::command::Command>> = Vec::new();
        for n in spec.nodes {
            cmds.push(Box::new(CmdAddNode { node: n }));
        }
        for c in spec.connections {
            cmds.push(Box::new(CmdSetConnection::new(c, true)));
        }
        cmds.push(Box::new(CmdAddNetworkBox {
            box_: spec.network_box,
        }));
        context.node_editor_state.execute(
            Box::new(CmdBatch::new("Create ForEach Primitive Block", cmds)),
            g,
        );
        context.ui_state.selected_nodes.clear();
        for id in &created {
            context.ui_state.selected_nodes.insert(*id);
        }
        context.ui_state.last_selected_node_id = created
            .get(1)
            .copied()
            .or(created.last().copied())
            .or(created.first().copied());
        context.ui_state.selected_network_boxes.clear();
        context.ui_state.selected_network_boxes.insert(bid);
    });
}

pub fn handle_menus(
    editor: &mut NodeEditorTab,
    ui: &mut egui::Ui,
    context: &mut EditorTabContext,
    response: &egui::Response,
    editor_rect: Rect,
) {
    let ctx = ui.ctx();
    let frame = ctx.cumulative_frame_nr();
    let search_popup_id = Id::new("ctx_search_popup");
    let node_popup_id = Id::new("ctx_node_popup");
    let cda_popup_id = Id::new("ctx_cda_popup");
    let (right_clicked, dbl_clicked, ptr_pos, esc) = ui.input(|i| {
        (
            i.pointer.secondary_clicked(),
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary),
            i.pointer.interact_pos(),
            i.key_pressed(egui::Key::Escape),
        )
    });

    // Right-click: lock node_id for subsequent frames
    if right_clicked {
        if let Some(pos) = ptr_pos {
            if editor_rect.contains(pos) {
                // Ensure old focused widgets (e.g. search input) don't keep stealing keyboard.
                ctx.memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
                editor.menu_state = if let Some(nid) = hit_node_at(editor, pos, editor_rect) {
                    // CDA nodes need a dedicated popup (keeps behavior predictable and avoids mixing with generic node menu).
                    let is_cda = {
                        let root = &context.node_graph_res.0;
                        cda::navigation::with_current_graph(&root, &editor.cda_state, |g| {
                            g.nodes.get(&nid).is_some_and(|n| {
                                matches!(
                                    n.node_type,
                                    NodeType::CDA(_)
                                        | NodeType::CDAInput(_)
                                        | NodeType::CDAOutput(_)
                                )
                            })
                        })
                    };
                    if is_cda {
                        ctx.memory_mut(|m| {
                            m.close_popup(search_popup_id);
                            m.close_popup(node_popup_id);
                            m.open_popup(cda_popup_id);
                        });
                        MenuState::CdaNode {
                            node_id: nid,
                            pos,
                            open_frame: frame,
                        }
                    } else {
                        ctx.memory_mut(|m| {
                            m.close_popup(search_popup_id);
                            m.close_popup(cda_popup_id);
                            m.open_popup(node_popup_id);
                        });
                        MenuState::Node {
                            node_id: nid,
                            pos,
                            open_frame: frame,
                        }
                    }
                } else {
                    editor.menu_search_text.clear();
                    editor.menu_search_cached_query.clear();
                    editor.menu_search_cached_results.clear();
                    editor.menu_search_cached_categories = context.node_registry.list_categories();
                    ctx.memory_mut(|m| {
                        m.close_popup(node_popup_id);
                        m.close_popup(cda_popup_id);
                        m.open_popup(search_popup_id);
                    });
                    MenuState::Search {
                        pos,
                        open_frame: frame,
                        context: None,
                    }
                };
            }
        }
    }

    // Double-click
    if dbl_clicked {
        if let Some(pos) = response.interact_pointer_pos() {
            if let Some(nid) = hit_node_at(editor, pos, editor_rect) {
                let root_graph = &context.node_graph_res.0;
                let path = editor.cda_state.breadcrumb();
                let entered =
                    cda::navigation::with_graph_by_path(&root_graph, &path, |display_graph| {
                        crate::tabs_system::node_editor::cda::navigation::handle_double_click_enter(
                            nid,
                            display_graph,
                            &mut editor.cda_state,
                            editor.pan,
                            editor.zoom,
                            editor_rect.size(),
                        )
                    });
                if let Some((new_pan, new_zoom)) = entered {
                    editor.pan = new_pan;
                    editor.zoom = new_zoom;
                    editor.target_pan = new_pan;
                    editor.target_zoom = new_zoom;
                    context.node_editor_state.pan = new_pan;
                    context.node_editor_state.zoom = new_zoom;
                    context.node_editor_state.target_pan = new_pan;
                    context.node_editor_state.target_zoom = new_zoom;
                    editor.cached_nodes_rev = 0; // Force refresh cache
                    editor.node_animations.clear();
                    editor.insertion_target = None;
                    editor.pending_connection_from = None;
                    editor.snapped_to_port = None;
                    close_menu(ctx, editor, None);
                    context.ui_state.selected_nodes.clear();
                    context.ui_state.selected_connections.clear();
                    context.ui_invalidator.request_repaint_after_tagged(
                        "node_editor/cda_enter",
                        Duration::ZERO,
                        RepaintCause::DataChanged,
                    );
                }
            } else {
                editor.menu_search_text.clear();
                editor.menu_search_cached_query.clear();
                editor.menu_search_cached_results.clear();
                editor.menu_search_cached_categories = context.node_registry.list_categories();
                ctx.memory_mut(|m| {
                    m.close_popup(node_popup_id);
                    m.open_popup(search_popup_id);
                });
                editor.menu_state = MenuState::Search {
                    pos,
                    open_frame: frame,
                    context: None,
                };
            }
        }
    }

    if esc {
        ctx.memory_mut(|m| {
            m.close_popup(search_popup_id);
            m.close_popup(node_popup_id);
            m.close_popup(cda_popup_id);
        });
        close_menu(ctx, editor, None);
    }

    // Draw
    let state = editor.menu_state.clone();
    // Reactive mode: keep hover/highlight stable, but cap redraw to avoid busy-loop.
    if !matches!(state, MenuState::None) {
        context.ui_invalidator.request_repaint_after_tagged(
            "node_editor/menu_open",
            Duration::from_secs_f32(1.0 / 60.0),
            RepaintCause::Input,
        );
    }
    match state {
        MenuState::None => {}
        MenuState::Search {
            pos,
            open_frame,
            context: _,
        } => {
            let spawn = (pos - editor_rect.min - editor.pan) / editor.zoom;
            let mut close = false;
            let search_id = Id::new("ctx_search_input");
            let inner = egui::Popup::new(
                Id::new("ctx_search_popup"),
                ctx.clone(),
                pos,
                egui::LayerId::new(egui::Order::Foreground, Id::new("ctx_search_popup")),
            )
            .kind(egui::PopupKind::Menu)
            .anchor(pos)
            .align(egui::RectAlign::TOP_START)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .open_memory(None::<egui::SetOpenCommand>)
            .show(|ui| {
                ui.set_max_width(220.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut editor.menu_search_text)
                        .id_source(search_id)
                        .hint_text("🔍 Search...")
                        .desired_width(200.0),
                );
                if !ui.memory(|m| m.has_focus(search_id)) {
                    r.request_focus();
                }
                ui.separator();
                let q = editor.menu_search_text.to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        if q.is_empty() {
                            if editor.menu_search_cached_categories.is_empty() {
                                editor.menu_search_cached_categories =
                                    context.node_registry.list_categories();
                            }
                            let cats = std::mem::take(&mut editor.menu_search_cached_categories);
                            for c in cats.iter() {
                                ui.menu_button(c.as_str(), |ui| {
                                    for (label, key) in
                                        context.node_registry.list_nodes_in_category_ui(c)
                                    {
                                        if ui.button(&label).clicked() {
                                            if key == "ForEach Begin" || key == "ForEach End" {
                                                spawn_foreach_block(
                                                    editor,
                                                    context,
                                                    spawn.to_pos2(),
                                                    key == "ForEach End",
                                                );
                                            } else if key == "ForEach Connectivity" {
                                                spawn_foreach_connectivity_block(
                                                    editor,
                                                    context,
                                                    spawn.to_pos2(),
                                                );
                                            } else if key == "ForEach Point" {
                                                spawn_foreach_point_block(
                                                    editor,
                                                    context,
                                                    spawn.to_pos2(),
                                                );
                                            } else if key == "ForEach Primitive" {
                                                spawn_foreach_primitive_block(
                                                    editor,
                                                    context,
                                                    spawn.to_pos2(),
                                                );
                                            } else {
                                                let root = &mut context.node_graph_res.0;
                                                cda::navigation::with_current_graph_mut(
                                                    root,
                                                    &editor.cda_state,
                                                    |g| {
                                                        let node = prepare_generic_node(
                                                            context.node_registry,
                                                            context.node_editor_settings,
                                                            spawn.to_pos2(),
                                                            &key,
                                                        );
                                                        let id = node.id;
                                                        context.node_editor_state.execute(
                                                            Box::new(CmdAddNode { node }),
                                                            g,
                                                        );
                                                        context.ui_state.selected_nodes.clear();
                                                        context.ui_state.selected_nodes.insert(id);
                                                        context.ui_state.last_selected_node_id =
                                                            Some(id);
                                                    },
                                                );
                                            }
                                            editor.cached_nodes_rev = 0;
                                            editor.geometry_rev =
                                                editor.geometry_rev.wrapping_add(1);
                                            context.graph_changed_writer.write_default();
                                            context.ui_invalidator.request_repaint_after_tagged(
                                                "ne",
                                                Duration::ZERO,
                                                RepaintCause::DataChanged,
                                            );
                                            close = true;
                                            ui.close();
                                        }
                                    }
                                });
                            }
                            editor.menu_search_cached_categories = cats;
                            if let Some(lib) =
                                crate::cunning_core::cda::library::global_cda_library()
                            {
                                let defs = lib.list_defs();
                                if !defs.is_empty() {
                                    ui.separator();
                                    ui.label(egui::RichText::new("CDA Library").weak());
                                    for (uuid, nm) in defs {
                                        if ui.button(format!("CDA: {}", nm)).clicked() {
                                            let path = lib.path_for(uuid).unwrap_or_default();
                                            let defaults = lib
                                                .get(uuid)
                                                .map(|a| {
                                                    a.coverlay_units
                                                        .iter()
                                                        .filter(|u| u.default_on)
                                                        .map(|u| u.node_id)
                                                        .collect()
                                                })
                                                .unwrap_or_else(Vec::new);
                                            let mut node = Node::new(
                                                uuid::Uuid::new_v4(),
                                                nm.clone(),
                                                NodeType::CDA(CDANodeData {
                                                    asset_ref:
                                                        crate::cunning_core::cda::CdaAssetRef {
                                                            uuid,
                                                            path,
                                                        },
                                                    name: nm.clone(),
                                                    coverlay_hud: None,
                                                    coverlay_units: defaults,
                                                    inner_param_overrides: Default::default(),
                                                }),
                                                spawn.to_pos2(),
                                            );
                                            let sz =
                                                crate::node_editor_settings::resolved_node_size(
                                                    context.node_editor_settings,
                                                );
                                            node.size = egui::vec2(sz[0], sz[1]);
                                            node.rebuild_parameters();
                                            let id = node.id;
                                            let root = &mut context.node_graph_res.0;
                                            cda::navigation::with_current_graph_mut(
                                                root,
                                                &editor.cda_state,
                                                |g| {
                                                    context
                                                        .node_editor_state
                                                        .execute(Box::new(CmdAddNode { node }), g);
                                                },
                                            );
                                            context.ui_state.selected_nodes.clear();
                                            context.ui_state.selected_nodes.insert(id);
                                            context.ui_state.last_selected_node_id = Some(id);
                                            editor.cached_nodes_rev = 0;
                                            editor.geometry_rev =
                                                editor.geometry_rev.wrapping_add(1);
                                            context.graph_changed_writer.write_default();
                                            context.ui_invalidator.request_repaint_after_tagged(
                                                "ne",
                                                Duration::ZERO,
                                                RepaintCause::DataChanged,
                                            );
                                            close = true;
                                            ui.close();
                                        }
                                    }
                                }
                            }
                        } else {
                            let mut found = false;
                            if editor.menu_search_cached_query != q {
                                editor.menu_search_cached_query = q.clone();
                                editor.menu_search_cached_results =
                                    context.node_registry.search_nodes_ui(&q);
                            }
                            let items = std::mem::take(&mut editor.menu_search_cached_results);
                            for (label, key) in items.iter() {
                                found = true;
                                if ui.button(label).clicked() {
                                    if *key == "ForEach Begin" || *key == "ForEach End" {
                                        spawn_foreach_block(
                                            editor,
                                            context,
                                            spawn.to_pos2(),
                                            *key == "ForEach End",
                                        );
                                    } else if *key == "ForEach Connectivity" {
                                        spawn_foreach_connectivity_block(
                                            editor,
                                            context,
                                            spawn.to_pos2(),
                                        );
                                    } else if *key == "ForEach Point" {
                                        spawn_foreach_point_block(editor, context, spawn.to_pos2());
                                    } else if *key == "ForEach Primitive" {
                                        spawn_foreach_primitive_block(
                                            editor,
                                            context,
                                            spawn.to_pos2(),
                                        );
                                    } else {
                                        let root = &mut context.node_graph_res.0;
                                        cda::navigation::with_current_graph_mut(
                                            root,
                                            &editor.cda_state,
                                            |g| {
                                                let node = prepare_generic_node(
                                                    context.node_registry,
                                                    context.node_editor_settings,
                                                    spawn.to_pos2(),
                                                    key,
                                                );
                                                let id = node.id;
                                                context
                                                    .node_editor_state
                                                    .execute(Box::new(CmdAddNode { node }), g);
                                                context.ui_state.selected_nodes.clear();
                                                context.ui_state.selected_nodes.insert(id);
                                                context.ui_state.last_selected_node_id = Some(id);
                                            },
                                        );
                                    }
                                    editor.cached_nodes_rev = 0;
                                    editor.geometry_rev = editor.geometry_rev.wrapping_add(1);
                                    context.graph_changed_writer.write_default();
                                    context.ui_invalidator.request_repaint_after_tagged(
                                        "ne",
                                        Duration::ZERO,
                                        RepaintCause::DataChanged,
                                    );
                                    close = true;
                                    ui.close();
                                }
                            }
                            editor.menu_search_cached_results = items;
                            if let Some(lib) =
                                crate::cunning_core::cda::library::global_cda_library()
                            {
                                for (uuid, nm) in lib.list_defs() {
                                    if nm.to_lowercase().contains(&q) {
                                        found = true;
                                        if ui.button(format!("CDA: {}", nm)).clicked() {
                                            let path = lib.path_for(uuid).unwrap_or_default();
                                            let defaults = lib
                                                .get(uuid)
                                                .map(|a| {
                                                    a.coverlay_units
                                                        .iter()
                                                        .filter(|u| u.default_on)
                                                        .map(|u| u.node_id)
                                                        .collect()
                                                })
                                                .unwrap_or_else(Vec::new);
                                            let mut node = Node::new(
                                                uuid::Uuid::new_v4(),
                                                nm.clone(),
                                                NodeType::CDA(CDANodeData {
                                                    asset_ref:
                                                        crate::cunning_core::cda::CdaAssetRef {
                                                            uuid,
                                                            path,
                                                        },
                                                    name: nm.clone(),
                                                    coverlay_hud: None,
                                                    coverlay_units: defaults,
                                                    inner_param_overrides: Default::default(),
                                                }),
                                                spawn.to_pos2(),
                                            );
                                            let sz =
                                                crate::node_editor_settings::resolved_node_size(
                                                    context.node_editor_settings,
                                                );
                                            node.size = egui::vec2(sz[0], sz[1]);
                                            node.rebuild_parameters();
                                            let id = node.id;
                                            let root = &mut context.node_graph_res.0;
                                            cda::navigation::with_current_graph_mut(
                                                root,
                                                &editor.cda_state,
                                                |g| {
                                                    context
                                                        .node_editor_state
                                                        .execute(Box::new(CmdAddNode { node }), g);
                                                },
                                            );
                                            context.ui_state.selected_nodes.clear();
                                            context.ui_state.selected_nodes.insert(id);
                                            context.ui_state.last_selected_node_id = Some(id);
                                            editor.cached_nodes_rev = 0;
                                            editor.geometry_rev =
                                                editor.geometry_rev.wrapping_add(1);
                                            context.graph_changed_writer.write_default();
                                            context.ui_invalidator.request_repaint_after_tagged(
                                                "ne",
                                                Duration::ZERO,
                                                RepaintCause::DataChanged,
                                            );
                                            close = true;
                                            ui.close();
                                        }
                                    }
                                }
                            }
                            if !found {
                                ui.label(egui::RichText::new("No nodes found").weak().italics());
                            }
                        }
                    });
            });
            let should_close = inner.as_ref().is_some_and(|ir| ir.response.should_close());
            if (frame != open_frame && should_close) || close || inner.is_none() {
                ctx.memory_mut(|m| m.close_popup(Id::new("ctx_search_popup")));
                close_menu(ctx, editor, Some(search_id));
            }
        }
        MenuState::CdaNode {
            node_id,
            pos,
            open_frame,
        } => {
            let root_graph = &context.node_graph_res.0;
            let Some((is_cda_inst, is_cda_in, is_cda_out, name)) =
                cda::navigation::with_current_graph(&root_graph, &editor.cda_state, |g| {
                    g.nodes.get(&node_id).map(|n| {
                        (
                            matches!(n.node_type, NodeType::CDA(_)),
                            matches!(n.node_type, NodeType::CDAInput(_)),
                            matches!(n.node_type, NodeType::CDAOutput(_)),
                            n.name.clone(),
                        )
                    })
                })
            else {
                close_menu(ctx, editor, None);
                return;
            };

            let mut close = false;
            let inner = egui::Popup::new(
                Id::new("ctx_cda_popup"),
                ctx.clone(),
                pos,
                egui::LayerId::new(egui::Order::Foreground, Id::new("ctx_cda_popup")),
            )
                .kind(egui::PopupKind::Menu)
                .anchor(pos)
                .align(egui::RectAlign::TOP_START)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .open_memory(None::<egui::SetOpenCommand>)
                .show(|ui| {
                    ui.set_min_width(180.0);
                    ui.set_max_width(320.0);
                    egui::ScrollArea::vertical().max_height(440.0).show(ui, |ui| {
                        let hdr = if is_cda_inst { format!("⬡ {}", name) } else if is_cda_in { format!("⬡ In: {}", name) } else if is_cda_out { format!("⬡ Out: {}", name) } else { format!("⬡ {}", name) };
                        ui.label(egui::RichText::new(hdr).strong());
                        ui.separator();

                        if is_cda_inst {
                            if ui.button("📝 Properties").clicked() { editor.cda_state.open_property_window(node_id); close = true; }
                            if ui.button("🔍 Enter Edit").clicked() {
                                let root_graph = &context.node_graph_res.0;
                                let path = editor.cda_state.breadcrumb();
                                let entered = cda::navigation::with_graph_by_path(&root_graph, &path, |display_graph| crate::tabs_system::node_editor::cda::navigation::handle_double_click_enter(node_id, display_graph, &mut editor.cda_state, editor.pan, editor.zoom, editor_rect.size()));
                                if let Some((new_pan, new_zoom)) = entered {
                                    editor.pan = new_pan;
                                    editor.zoom = new_zoom;
                                    editor.target_pan = new_pan;
                                    editor.target_zoom = new_zoom;
                                    context.node_editor_state.pan = new_pan;
                                    context.node_editor_state.zoom = new_zoom;
                                    context.node_editor_state.target_pan = new_pan;
                                    context.node_editor_state.target_zoom = new_zoom;
                                    editor.cached_nodes_rev = 0;
                                    editor.node_animations.clear();
                                    editor.insertion_target = None;
                                    editor.pending_connection_from = None;
                                    editor.snapped_to_port = None;
                                    context.ui_state.selected_nodes.clear();
                                    context.ui_state.selected_connections.clear();
                                    context.ui_invalidator.request_repaint_after_tagged("node_editor/cda_enter", Duration::ZERO, RepaintCause::DataChanged);
                                }
                                close = true;
                            }
                            ui.separator();
                            if ui.button("💾 Save .cda").clicked() {
                                let (asset_uuid, default_name) = {
                                    let root = &context.node_graph_res.0;
                                    let g = cda::navigation::graph_snapshot_by_path(&root, &editor.cda_state.breadcrumb());
                                    if let Some(n) = g.nodes.get(&node_id) {
                                        if let NodeType::CDA(data) = &n.node_type {
                                            (Some(data.asset_ref.uuid), if data.name.is_empty() { "asset".to_string() } else { data.name.clone() })
                                        } else {
                                            (None, "asset".to_string())
                                        }
                                    } else {
                                        (None, "asset".to_string())
                                    }
                                };
                                if let Some(asset_uuid) = asset_uuid {
                                    if let Some(p) = FileDialog::new().add_filter("CDA", &["cda"]).set_file_name(&format!("{}.cda", default_name)).save_file() {
                                        let mut saved = false;
                                        if let Some(lib) = crate::cunning_core::cda::library::global_cda_library() {
                                            if let Some(asset) = lib.get(asset_uuid) {
                                                let mut asset = asset.clone();
                                                asset.normalize_ports_for_runtime();
                                                match asset.save_with_report(p.clone()) {
                                                    Ok(_) => { saved = true; context.console_log.info(format!("Saved CDA: {} ({}) -> {:?}", asset.name, asset.id, p)); }
                                                    Err(e) => context.console_log.error(format!("Save CDA failed: {:?} ({})", e, asset.name)),
                                                }
                                            } else {
                                                context.console_log.error(format!("Save CDA failed: asset not found in library ({})", asset_uuid));
                                            }
                                        } else {
                                            context.console_log.error("Save CDA failed: CdaLibrary not initialized".to_string());
                                        }
                                        if saved {
                                            let path_str = p.to_string_lossy().to_string();
                                            let root = &mut context.node_graph_res.0;
                                            cda::navigation::with_current_graph_mut(root, &editor.cda_state, |g| {
                                                if let Some(n) = g.nodes.get_mut(&node_id) {
                                                    if let NodeType::CDA(d) = &mut n.node_type { d.asset_ref.path = path_str.clone(); }
                                                }
                                            });
                                        }
                                    }
                                }
                                close = true;
                            }
                            if ui.button("📦 Unpack").clicked() {
                                let root = &mut context.node_graph_res.0;
                                cda::navigation::with_current_graph_mut(root, &editor.cda_state, |g| { let before=GraphSnapshot::capture(g); super::cda_context::unpack_cda(g, node_id); let after=GraphSnapshot::capture(g); context.node_editor_state.record(Box::new(CmdReplaceGraph::new(before, after))); });
                                context.graph_changed_writer.write_default();
                                close = true;
                            }
                        } else {
                            // CDA IO nodes: keep minimal but distinct menu (these are definition-scoped nodes).
                            ui.label(egui::RichText::new("CDA I/O Node").weak());
                            ui.separator();
                            if ui.button("🗑️ Delete").clicked() {
                                let root = &mut context.node_graph_res.0;
                                cda::navigation::with_current_graph_mut(root, &editor.cda_state, |g| context.node_editor_state.execute(Box::new(CmdDeleteNodes::new(vec![node_id])), g));
                                context.ui_state.selected_nodes.remove(&node_id);
                                context.graph_changed_writer.write_default();
                                close = true;
                            }
                        }
                    });
                    if close { ui.close(); }
                });
            let should_close = inner.as_ref().is_some_and(|ir| ir.response.should_close());
            if (frame != open_frame && should_close) || close || inner.is_none() {
                ctx.memory_mut(|m| m.close_popup(Id::new("ctx_cda_popup")));
                close_menu(ctx, editor, None);
            }
        }
        MenuState::Node {
            node_id,
            pos,
            open_frame,
        } => {
            let root_graph = &context.node_graph_res.0;
            let Some((is_cda, is_foreach, foreach_block_id, name, tname)) =
                cda::navigation::with_current_graph(&root_graph, &editor.cda_state, |g| {
                    g.nodes.get(&node_id).map(|node| {
                let is_cda = matches!(node.node_type, NodeType::CDA(_));
                let is_foreach = matches!(&node.node_type, NodeType::Generic(s) if s == "ForEach Begin" || s == "ForEach End");
                let foreach_block_id = if is_foreach { node.parameters.iter().find(|p| p.name == "block_id").and_then(|p| if let crate::nodes::parameter::ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None }).unwrap_or_default() } else { String::new() };
                (is_cda, is_foreach, foreach_block_id, node.name.clone(), node.node_type.name().to_string())
            })
                })
            else {
                close_menu(ctx, editor, None);
                return;
            };

            let mut close = false;
            let inner = egui::Popup::new(
                Id::new("ctx_node_popup"),
                ctx.clone(),
                pos,
                egui::LayerId::new(egui::Order::Foreground, Id::new("ctx_node_popup")),
            )
                .kind(egui::PopupKind::Menu)
                .anchor(pos)
                .align(egui::RectAlign::TOP_START)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .open_memory(None::<egui::SetOpenCommand>)
                .show(|ui| {
                    ui.set_min_width(160.0);
                    ui.set_max_width(280.0);
                    egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                        if is_cda {
                        ui.label(egui::RichText::new(format!("⬡ {}", name)).strong());
                        ui.separator();
                        if ui.button("📝 Properties").clicked() { editor.cda_state.open_property_window(node_id); close=true; }
                        if ui.button("🔍 Enter Edit").clicked() { 
                            let root_graph = &context.node_graph_res.0;
                            let path = editor.cda_state.breadcrumb();
                            let entered = cda::navigation::with_graph_by_path(&root_graph, &path, |display_graph| crate::tabs_system::node_editor::cda::navigation::handle_double_click_enter(node_id, display_graph, &mut editor.cda_state, editor.pan, editor.zoom, editor_rect.size()));
                            if let Some((new_pan, new_zoom)) = entered {
                                editor.pan = new_pan;
                                editor.zoom = new_zoom;
                                editor.target_pan = new_pan;
                                editor.target_zoom = new_zoom;
                                context.node_editor_state.pan = new_pan;
                                context.node_editor_state.zoom = new_zoom;
                                context.node_editor_state.target_pan = new_pan;
                                context.node_editor_state.target_zoom = new_zoom;
                                editor.cached_nodes_rev = 0;
                                editor.node_animations.clear();
                                editor.insertion_target = None;
                                editor.pending_connection_from = None;
                                editor.snapped_to_port = None;
                                context.ui_state.selected_nodes.clear();
                                context.ui_state.selected_connections.clear();
                                context.ui_invalidator.request_repaint_after_tagged("node_editor/cda_enter", Duration::ZERO, RepaintCause::DataChanged);
                            }
                            close=true; 
                        }
                        ui.separator();
                            if ui.button("💾 Save .cda").clicked() {
                            let (asset_uuid, default_name) = {
                                let root = &context.node_graph_res.0;
                                let g = cda::navigation::graph_snapshot_by_path(&root, &editor.cda_state.breadcrumb());
                                if let Some(n) = g.nodes.get(&node_id) {
                                    if let NodeType::CDA(data) = &n.node_type {
                                        (Some(data.asset_ref.uuid), if data.name.is_empty() { "asset".to_string() } else { data.name.clone() })
                                    } else {
                                        (None, "asset".to_string())
                                    }
                                } else {
                                    (None, "asset".to_string())
                                }
                            };
                            if let Some(asset_uuid) = asset_uuid {
                                if let Some(p) = FileDialog::new().add_filter("CDA", &["cda"]).set_file_name(&format!("{}.cda", default_name)).save_file() {
                                    let mut saved = false;
                                    if let Some(lib) = crate::cunning_core::cda::library::global_cda_library() {
                                        if let Some(asset) = lib.get(asset_uuid) {
                                            // Converge to stable port keys before saving (no mixed label/key ports on disk).
                                            let mut asset = asset.clone();
                                            asset.normalize_ports_for_runtime();
                                            match asset.save_with_report(p.clone()) {
                                                Ok(_) => { saved = true; context.console_log.info(format!("Saved CDA: {} ({}) -> {:?}", asset.name, asset.id, p)); }
                                                Err(e) => context.console_log.error(format!("Save CDA failed: {:?} ({})", e, asset.name)),
                                            }
                                        } else {
                                            context.console_log.error(format!("Save CDA failed: asset not found in library ({})", asset_uuid));
                                        }
                                    } else {
                                        context.console_log.error("Save CDA failed: CdaLibrary not initialized".to_string());
                                    }
                                    if saved {
                                    let path_str = p.to_string_lossy().to_string();
                                    let root = &mut context.node_graph_res.0;
                                    cda::navigation::with_current_graph_mut(root, &editor.cda_state, |g| {
                                        if let Some(n) = g.nodes.get_mut(&node_id) {
                                            if let NodeType::CDA(d) = &mut n.node_type { d.asset_ref.path = path_str.clone(); }
                                        }
                                    });
                                    }
                                }
                            }
                            close=true;
                        }
                        if ui.button("📦 Unpack").clicked() { let root=&mut context.node_graph_res.0; cda::navigation::with_current_graph_mut(root, &editor.cda_state, |g| { let before=GraphSnapshot::capture(g); super::cda_context::unpack_cda(g, node_id); let after=GraphSnapshot::capture(g); context.node_editor_state.record(Box::new(CmdReplaceGraph::new(before, after))); }); context.graph_changed_writer.write_default(); close=true; }
                    } else {
                        ui.label(egui::RichText::new(format!("{} ({})", name, tname)).strong());
                        ui.separator();
                        if ui.button("📋 Copy").clicked() { let root=&mut context.node_graph_res.0; cda::navigation::with_current_graph_mut(root, &editor.cda_state, |g| { if let Some(n)=g.nodes.get(&node_id){editor.copied_nodes=vec![n.clone()];} }); close=true; }
                        if ui.button("🗑️ Delete").clicked() { let root=&mut context.node_graph_res.0; cda::navigation::with_current_graph_mut(root, &editor.cda_state, |g| context.node_editor_state.execute(Box::new(CmdDeleteNodes::new(vec![node_id])), g)); context.ui_state.selected_nodes.remove(&node_id); context.graph_changed_writer.write_default(); close=true; }
                        ui.separator();
                        if ui.button("🔄 Recalculate").clicked() { let root=&mut context.node_graph_res.0; cda::navigation::with_current_graph_mut(root, &editor.cda_state, |g| g.mark_dirty(node_id)); context.graph_changed_writer.write_default(); close=true; }
                        if ui.button("🧠 Generate Coverlay Panel").clicked() {
                            editor.coverlay_gen_open = true;
                            editor.coverlay_gen_anchor = Some(pos);
                            if editor.coverlay_gen_prompt.trim().is_empty() {
                                editor.coverlay_gen_prompt = "Create a compact control panel for the selected nodes. Prefer the most important parameters. Output bindings_json only.".to_string();
                            }
                            close = true;
                        }
                        if is_foreach && ui.button("🧹 Reset ForEach Cache").clicked() {
                            let bid = foreach_block_id.clone();
                            let root=&mut context.node_graph_res.0;
                            cda::navigation::with_current_graph_mut(root, &editor.cda_state, |g| g.reset_foreach_cache_by_block_id(&bid));
                            context.graph_changed_writer.write_default();
                            close=true;
                        }
                    }
                    });
                    if close { ui.close(); }
                });
            let should_close = inner.as_ref().is_some_and(|ir| ir.response.should_close());
            if (frame != open_frame && should_close) || close || inner.is_none() {
                ctx.memory_mut(|m| m.close_popup(Id::new("ctx_node_popup")));
                close_menu(ctx, editor, None);
            }
        }
    }
}
