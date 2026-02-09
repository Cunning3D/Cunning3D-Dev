//! Bevy UI Shelf (Desktop) - cgui-based Houdini-style shelf toolbar
use crate::launcher::plugin::AppState;
use crate::timeline_bevy_ui::TimelineUiCamera;
use crate::ui::{ShelfCommand, ShelfTab, UiState};
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::ui::prelude as cgui;
use bevy::ui::{FocusPolicy, UiSystems};
use bevy_egui::EguiHole;

#[derive(Component)]
struct ShelfUiRoot;
#[derive(Component)]
struct ShelfBar;
#[derive(Component)]
struct ShelfSetRoot {
    surface: egui_dock::SurfaceIndex,
    node: egui_dock::NodeIndex,
}
#[derive(Component)]
struct ShelfTabBtn {
    surface: egui_dock::SurfaceIndex,
    node: egui_dock::NodeIndex,
    tab: ShelfTab,
    idx: egui_dock::TabIndex,
}
#[derive(Component)]
struct ShelfAddBtn {
    surface: egui_dock::SurfaceIndex,
    node: egui_dock::NodeIndex,
}
#[derive(Component)]
struct ShelfPopup {
    surface: egui_dock::SurfaceIndex,
    node: egui_dock::NodeIndex,
}
#[derive(Component)]
struct ShelfPopupItemNewSet {
    surface: egui_dock::SurfaceIndex,
    node: egui_dock::NodeIndex,
}
#[derive(Component)]
struct ShelfPopupItemToggleTab {
    surface: egui_dock::SurfaceIndex,
    node: egui_dock::NodeIndex,
    tab: ShelfTab,
}
#[derive(Component)]
struct ShelfToolBtn(ShelfTool);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShelfTool {
    Cube,
    Sphere,
    Transform,
    Promote,
    Merge,
    Dummy(&'static str),
}

#[derive(Resource, Default)]
struct ShelfMenuState {
    open: Option<(egui_dock::SurfaceIndex, egui_dock::NodeIndex)>,
}

#[derive(Resource, Default)]
struct ShelfUiSpawnGate {
    spawned: bool,
}

pub struct ShelfUiPlugin;

const UI_LAYER: usize = 31;
const TOPBAR_H: f32 = 28.0;
const SHELF_H: f32 = 80.0;

impl Plugin for ShelfUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShelfMenuState>()
            .init_resource::<ShelfUiSpawnGate>()
            .add_systems(OnEnter(AppState::Editor), reset_shelf_spawn_gate)
            .add_systems(
                Update,
                (spawn_shelf_ui_once, sync_shelf_ui, update_tool_btn_hover)
                    .run_if(in_state(AppState::Editor)),
            )
            .add_systems(
                PreUpdate,
                (
                    handle_shelf_clicks,
                    close_popup_on_esc,
                    apply_shelf_commands,
                )
                    .after(UiSystems::Focus)
                    .run_if(in_state(AppState::Editor)),
            );
    }
}

fn reset_shelf_spawn_gate(mut g: ResMut<ShelfUiSpawnGate>) {
    *g = default();
}

fn spawn_shelf_ui_once(
    mut commands: Commands,
    mut g: ResMut<ShelfUiSpawnGate>,
    cam_q: Query<Entity, With<TimelineUiCamera>>,
    existing: Query<Entity, With<ShelfUiRoot>>,
) {
    if g.spawned || !existing.is_empty() {
        return;
    }
    let Some(cam) = cam_q.iter().next() else {
        return;
    };
    g.spawned = true;

    let root = commands
        .spawn_empty()
        .insert(ShelfUiRoot)
        .insert(cgui::UiTargetCamera(cam))
        .insert(RenderLayers::layer(UI_LAYER))
        .insert(cgui::GlobalZIndex(8))
        .insert(cgui::ZIndex(0))
        .insert(cgui::Node {
            position_type: cgui::PositionType::Absolute,
            left: cgui::Val::Px(0.0),
            right: cgui::Val::Px(0.0),
            top: cgui::Val::Px(0.0),
            height: cgui::Val::Px(TOPBAR_H + SHELF_H),
            ..default()
        })
        .id();

    let bar = commands
        .spawn_empty()
        .insert(ShelfBar)
        .insert(EguiHole)
        .insert(RenderLayers::layer(UI_LAYER))
        .insert(FocusPolicy::Pass)
        .insert(cgui::Interaction::None)
        .insert(cgui::Node {
            position_type: cgui::PositionType::Absolute,
            left: cgui::Val::Px(0.0),
            right: cgui::Val::Px(0.0),
            top: cgui::Val::Px(TOPBAR_H),
            height: cgui::Val::Px(SHELF_H),
            flex_direction: cgui::FlexDirection::Row,
            align_items: cgui::AlignItems::Stretch,
            ..default()
        })
        .insert(cgui::BackgroundColor(Color::srgba(0.18, 0.18, 0.20, 0.98)))
        .id();

    commands.entity(root).add_child(bar);
}

fn leaf_list(
    state: &egui_dock::DockState<ShelfTab>,
) -> Vec<(egui_dock::SurfaceIndex, egui_dock::NodeIndex, f32)> {
    fn walk(
        st: &egui_dock::DockState<ShelfTab>,
        s: egui_dock::SurfaceIndex,
        n: egui_dock::NodeIndex,
        w: f32,
        out: &mut Vec<(egui_dock::SurfaceIndex, egui_dock::NodeIndex, f32)>,
    ) {
        if n.0 >= st[s].len() {
            return;
        }
        match &st[s][n] {
            egui_dock::Node::Leaf { .. } => out.push((s, n, w.max(0.0))),
            egui_dock::Node::Horizontal { fraction, .. } => {
                let f = (*fraction).clamp(0.05, 0.95);
                walk(st, s, n.left(), w * f, out);
                walk(st, s, n.right(), w * (1.0 - f), out);
            }
            egui_dock::Node::Vertical { fraction, .. } => {
                let f = (*fraction).clamp(0.05, 0.95);
                walk(st, s, n.left(), w * f, out);
                walk(st, s, n.right(), w * (1.0 - f), out);
            }
            egui_dock::Node::Empty => {}
        }
    }
    let mut out = Vec::new();
    let s = egui_dock::SurfaceIndex(0);
    walk(state, s, egui_dock::NodeIndex::root(), 1.0, &mut out);
    if out.is_empty() {
        out.push((s, egui_dock::NodeIndex::root(), 1.0));
    }
    out
}

fn tab_list_in_leaf(
    state: &egui_dock::DockState<ShelfTab>,
    surface: egui_dock::SurfaceIndex,
    node: egui_dock::NodeIndex,
) -> (Vec<(ShelfTab, egui_dock::TabIndex)>, egui_dock::TabIndex) {
    match &state[surface][node] {
        egui_dock::Node::Leaf { tabs, active, .. } => (
            tabs.iter()
                .cloned()
                .enumerate()
                .map(|(i, t)| (t, egui_dock::TabIndex(i)))
                .collect(),
            *active,
        ),
        _ => (
            vec![(ShelfTab::Create, egui_dock::TabIndex(0))],
            egui_dock::TabIndex(0),
        ),
    }
}

fn set_active_tab(
    state: &mut egui_dock::DockState<ShelfTab>,
    surface: egui_dock::SurfaceIndex,
    node: egui_dock::NodeIndex,
    idx: egui_dock::TabIndex,
) {
    if let egui_dock::Node::Leaf { active, .. } = &mut state[surface][node] {
        *active = idx;
    }
}

fn has_tab(
    state: &egui_dock::DockState<ShelfTab>,
    surface: egui_dock::SurfaceIndex,
    node: egui_dock::NodeIndex,
    tab: ShelfTab,
) -> bool {
    match &state[surface][node] {
        egui_dock::Node::Leaf { tabs, .. } => tabs.iter().any(|t| *t == tab),
        _ => false,
    }
}

fn sync_shelf_ui(
    mut commands: Commands,
    ui_state: Res<UiState>,
    menu: Res<ShelfMenuState>,
    bar_q: Query<Entity, With<ShelfBar>>,
    children_q: Query<&Children>,
    mut last: Local<(u64, Option<(egui_dock::SurfaceIndex, egui_dock::NodeIndex)>)>,
) {
    let Some(bar) = bar_q.iter().next() else {
        return;
    };
    let leaves = leaf_list(&ui_state.shelf_dock_state);
    let mut sig = leaves.len() as u64;
    for (s, n, w) in leaves.iter() {
        sig ^= (s.0 as u64).wrapping_mul(0x9e3779b185ebca87);
        sig ^= (n.0 as u64).wrapping_mul(0xc2b2ae3d27d4eb4f);
        sig ^= (w.to_bits() as u64).wrapping_mul(0x165667b19e3779f9);
        if n.0 < ui_state.shelf_dock_state[*s].len() {
            if let egui_dock::Node::Leaf { tabs, active, .. } = &ui_state.shelf_dock_state[*s][*n] {
                sig ^= (tabs.len() as u64) << 12;
                sig ^= (active.0 as u64) << 20;
            }
        }
    }
    let cur = (sig, menu.open);
    if *last == cur {
        return;
    }
    *last = cur;

    if let Ok(kids) = children_q.get(bar) {
        for &c in kids {
            commands.entity(c).despawn();
        }
    }

    let sum_w: f32 = leaves.iter().map(|(_, _, w)| *w).sum::<f32>().max(0.001);
    let num_sets = leaves.len();
    for (i, (surface, node, w)) in leaves.iter().enumerate() {
        let (tabs, active) = tab_list_in_leaf(&ui_state.shelf_dock_state, *surface, *node);
        let active_tab = tabs
            .iter()
            .find(|(_, idx)| *idx == active)
            .map(|(t, _)| t.clone())
            .unwrap_or(ShelfTab::Create);
        let open = menu.open == Some((*surface, *node));

        // Set container - VERTICAL layout (tab bar on top, tools below)
        let set = commands
            .spawn_empty()
            .insert(ShelfSetRoot {
                surface: *surface,
                node: *node,
            })
            .insert(RenderLayers::layer(UI_LAYER))
            .insert(cgui::Node {
                width: cgui::Val::Percent((w / sum_w) * 100.0),
                height: cgui::Val::Percent(100.0),
                flex_direction: cgui::FlexDirection::Column,
                padding: cgui::UiRect::all(cgui::Val::Px(4.0)),
                row_gap: cgui::Val::Px(2.0),
                border: if i < num_sets - 1 {
                    cgui::UiRect {
                        right: cgui::Val::Px(1.0),
                        ..default()
                    }
                } else {
                    cgui::UiRect::ZERO
                },
                ..default()
            })
            .insert(cgui::BackgroundColor(Color::srgba(0.14, 0.14, 0.16, 1.0)))
            .insert(cgui::BorderColor::from(Color::srgba(0.30, 0.30, 0.32, 0.9)))
            .id();

        // Tab bar (top row)
        let tab_bar = commands
            .spawn_empty()
            .insert(RenderLayers::layer(UI_LAYER))
            .insert(cgui::Node {
                height: cgui::Val::Px(22.0),
                flex_direction: cgui::FlexDirection::Row,
                align_items: cgui::AlignItems::Center,
                column_gap: cgui::Val::Px(2.0),
                ..default()
            })
            .id();
        commands.entity(set).add_child(tab_bar);

        // Tab buttons
        for (t, idx) in tabs.iter().cloned() {
            let sel = idx == active;
            let btn = commands
                .spawn_empty()
                .insert(ShelfTabBtn {
                    surface: *surface,
                    node: *node,
                    tab: t.clone(),
                    idx,
                })
                .insert(RenderLayers::layer(UI_LAYER))
                .insert(cgui::Button)
                .insert(cgui::Interaction::None)
                .insert(cgui::Node {
                    height: cgui::Val::Px(20.0),
                    padding: cgui::UiRect::horizontal(cgui::Val::Px(8.0)),
                    align_items: cgui::AlignItems::Center,
                    border_radius: cgui::BorderRadius::all(cgui::Val::Px(3.0)),
                    ..default()
                })
                .insert(cgui::BackgroundColor(if sel {
                    Color::srgba(0.38, 0.38, 0.42, 1.0)
                } else {
                    Color::srgba(0.22, 0.22, 0.25, 0.9)
                }))
                .id();
            let txt = commands
                .spawn((
                    cgui::Text::new(format!("{:?}", t)),
                    TextFont {
                        font_size: FontSize::Px(11.0),
                        ..default()
                    },
                    TextColor(if sel {
                        Color::WHITE
                    } else {
                        Color::srgba(0.82, 0.82, 0.82, 1.0)
                    }),
                    RenderLayers::layer(UI_LAYER),
                ))
                .id();
            commands.entity(btn).add_child(txt);
            commands.entity(tab_bar).add_child(btn);
        }

        // Add button (+)
        let add_btn = commands
            .spawn_empty()
            .insert(ShelfAddBtn {
                surface: *surface,
                node: *node,
            })
            .insert(RenderLayers::layer(UI_LAYER))
            .insert(cgui::Button)
            .insert(cgui::Interaction::None)
            .insert(cgui::Node {
                width: cgui::Val::Px(20.0),
                height: cgui::Val::Px(20.0),
                justify_content: cgui::JustifyContent::Center,
                align_items: cgui::AlignItems::Center,
                border_radius: cgui::BorderRadius::all(cgui::Val::Px(3.0)),
                margin: cgui::UiRect {
                    left: cgui::Val::Px(4.0),
                    ..default()
                },
                ..default()
            })
            .insert(cgui::BackgroundColor(Color::srgba(0.22, 0.22, 0.25, 0.9)))
            .id();
        let add_txt = commands
            .spawn((
                cgui::Text::new("▼"),
                TextFont {
                    font_size: FontSize::Px(10.0),
                    ..default()
                },
                TextColor(Color::srgba(0.88, 0.88, 0.88, 1.0)),
                RenderLayers::layer(UI_LAYER),
            ))
            .id();
        commands.entity(add_btn).add_child(add_txt);
        commands.entity(tab_bar).add_child(add_btn);

        // Tools area (bottom row, fills remaining height)
        let tools = commands
            .spawn_empty()
            .insert(RenderLayers::layer(UI_LAYER))
            .insert(cgui::Node {
                flex_grow: 1.0,
                width: cgui::Val::Percent(100.0),
                flex_direction: cgui::FlexDirection::Row,
                align_items: cgui::AlignItems::Center,
                column_gap: cgui::Val::Px(2.0),
                ..default()
            })
            .id();
        commands.entity(set).add_child(tools);

        for tool in tools_for_tab(active_tab) {
            let btn = commands
                .spawn_empty()
                .insert(ShelfToolBtn(*tool))
                .insert(RenderLayers::layer(UI_LAYER))
                .insert(cgui::Button)
                .insert(cgui::Interaction::None)
                .insert(cgui::Node {
                    width: cgui::Val::Px(54.0),
                    height: cgui::Val::Px(54.0),
                    flex_direction: cgui::FlexDirection::Column,
                    justify_content: cgui::JustifyContent::Center,
                    align_items: cgui::AlignItems::Center,
                    border_radius: cgui::BorderRadius::all(cgui::Val::Px(4.0)),
                    ..default()
                })
                .insert(cgui::BackgroundColor(Color::NONE))
                .id();
            let (icon, label) = tool_label(*tool);
            let icon_txt = commands
                .spawn((
                    cgui::Text::new(icon),
                    TextFont {
                        font_size: FontSize::Px(22.0),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    RenderLayers::layer(UI_LAYER),
                ))
                .id();
            let label_txt = commands
                .spawn((
                    cgui::Text::new(label),
                    TextFont {
                        font_size: FontSize::Px(10.0),
                        ..default()
                    },
                    TextColor(Color::srgba(0.72, 0.72, 0.72, 1.0)),
                    RenderLayers::layer(UI_LAYER),
                ))
                .id();
            commands.entity(btn).add_child(icon_txt);
            commands.entity(btn).add_child(label_txt);
            commands.entity(tools).add_child(btn);
        }

        // Popup menu (absolute positioned)
        let popup = commands
            .spawn_empty()
            .insert(ShelfPopup {
                surface: *surface,
                node: *node,
            })
            .insert(EguiHole)
            .insert(RenderLayers::layer(UI_LAYER))
            .insert(cgui::Interaction::None)
            .insert(FocusPolicy::Pass)
            .insert(cgui::GlobalZIndex(20))
            .insert(cgui::Node {
                position_type: cgui::PositionType::Absolute,
                left: cgui::Val::Px(4.0),
                top: cgui::Val::Px(26.0),
                width: cgui::Val::Px(160.0),
                padding: cgui::UiRect::all(cgui::Val::Px(4.0)),
                row_gap: cgui::Val::Px(1.0),
                flex_direction: cgui::FlexDirection::Column,
                border_radius: cgui::BorderRadius::all(cgui::Val::Px(4.0)),
                display: if open {
                    cgui::Display::Flex
                } else {
                    cgui::Display::None
                },
                ..default()
            })
            .insert(cgui::BackgroundColor(Color::srgba(0.16, 0.16, 0.18, 0.98)))
            .id();
        commands.entity(set).add_child(popup);

        // Popup: New Set
        let new_set_btn = commands
            .spawn_empty()
            .insert(ShelfPopupItemNewSet {
                surface: *surface,
                node: *node,
            })
            .insert(RenderLayers::layer(UI_LAYER))
            .insert(cgui::Button)
            .insert(cgui::Interaction::None)
            .insert(cgui::Node {
                height: cgui::Val::Px(22.0),
                padding: cgui::UiRect::horizontal(cgui::Val::Px(8.0)),
                align_items: cgui::AlignItems::Center,
                border_radius: cgui::BorderRadius::all(cgui::Val::Px(3.0)),
                ..default()
            })
            .insert(cgui::BackgroundColor(Color::srgba(0.22, 0.22, 0.24, 1.0)))
            .id();
        let new_set_txt = commands
            .spawn((
                cgui::Text::new("+ New Shelf Set"),
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgba(0.92, 0.92, 0.92, 1.0)),
                RenderLayers::layer(UI_LAYER),
            ))
            .id();
        commands.entity(new_set_btn).add_child(new_set_txt);
        commands.entity(popup).add_child(new_set_btn);

        let sep = commands
            .spawn((
                cgui::Node {
                    height: cgui::Val::Px(6.0),
                    ..default()
                },
                RenderLayers::layer(UI_LAYER),
            ))
            .id();
        commands.entity(popup).add_child(sep);

        // Popup: Tab toggles
        let all_tabs = [
            ShelfTab::Create,
            ShelfTab::Modify,
            ShelfTab::Model,
            ShelfTab::Polygon,
            ShelfTab::Deform,
            ShelfTab::Texture,
            ShelfTab::Rigging,
        ];
        for t in all_tabs {
            let present = has_tab(&ui_state.shelf_dock_state, *surface, *node, t.clone());
            let label = format!("{} {:?}", if present { "[x]" } else { "[ ]" }, t);
            let toggle_btn = commands
                .spawn_empty()
                .insert(ShelfPopupItemToggleTab {
                    surface: *surface,
                    node: *node,
                    tab: t,
                })
                .insert(RenderLayers::layer(UI_LAYER))
                .insert(cgui::Button)
                .insert(cgui::Interaction::None)
                .insert(cgui::Node {
                    height: cgui::Val::Px(20.0),
                    padding: cgui::UiRect::horizontal(cgui::Val::Px(8.0)),
                    align_items: cgui::AlignItems::Center,
                    border_radius: cgui::BorderRadius::all(cgui::Val::Px(3.0)),
                    ..default()
                })
                .insert(cgui::BackgroundColor(Color::srgba(0.20, 0.20, 0.22, 1.0)))
                .id();
            let toggle_txt = commands
                .spawn((
                    cgui::Text::new(label),
                    TextFont {
                        font_size: FontSize::Px(11.0),
                        ..default()
                    },
                    TextColor(Color::srgba(0.88, 0.88, 0.88, 1.0)),
                    RenderLayers::layer(UI_LAYER),
                ))
                .id();
            commands.entity(toggle_btn).add_child(toggle_txt);
            commands.entity(popup).add_child(toggle_btn);
        }

        commands.entity(bar).add_child(set);
    }
}

fn tools_for_tab(tab: ShelfTab) -> &'static [ShelfTool] {
    match tab {
        ShelfTab::Create => &[
            ShelfTool::Cube,
            ShelfTool::Sphere,
            ShelfTool::Transform,
            ShelfTool::Promote,
            ShelfTool::Merge,
            ShelfTool::Dummy("Grid"),
            ShelfTool::Dummy("Torus"),
            ShelfTool::Dummy("Tube"),
        ],
        ShelfTab::Modify => &[
            ShelfTool::Dummy("Edit"),
            ShelfTool::Dummy("Clip"),
            ShelfTool::Dummy("Extrude"),
            ShelfTool::Dummy("Facet"),
        ],
        ShelfTab::Deform => &[
            ShelfTool::Dummy("Bend"),
            ShelfTool::Dummy("Twist"),
            ShelfTool::Dummy("Noise"),
        ],
        ShelfTab::Rigging => &[
            ShelfTool::Dummy("Bone"),
            ShelfTool::Dummy("IK"),
            ShelfTool::Dummy("Weight"),
        ],
        _ => &[ShelfTool::Dummy("WIP")],
    }
}

fn tool_label(t: ShelfTool) -> (&'static str, &'static str) {
    match t {
        ShelfTool::Cube => ("[=]", "Cube"),
        ShelfTool::Sphere => ("( )", "Sphere"),
        ShelfTool::Transform => ("<+>", "Transform"),
        ShelfTool::Promote => ("[^]", "Promote"),
        ShelfTool::Merge => ("[+]", "Merge"),
        ShelfTool::Dummy(s) => match s {
            "Grid" => ("[#]", s),
            "Torus" => ("(O)", s),
            "Tube" => ("[|]", s),
            "Bend" => ("[~]", s),
            "Twist" => ("[%]", s),
            "Bone" => ("[I]", s),
            _ => ("[ ]", s),
        },
    }
}

// Hover/Pressed effect for tool buttons
fn update_tool_btn_hover(
    mut q: Query<
        (&cgui::Interaction, &mut cgui::BackgroundColor),
        (With<ShelfToolBtn>, Changed<cgui::Interaction>),
    >,
) {
    for (inter, mut bg) in &mut q {
        *bg = match inter {
            cgui::Interaction::Pressed => cgui::BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.18)),
            cgui::Interaction::Hovered => cgui::BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.10)),
            cgui::Interaction::None => cgui::BackgroundColor(Color::NONE),
        };
    }
}

fn handle_shelf_clicks(
    mut ui_state: ResMut<UiState>,
    mut menu: ResMut<ShelfMenuState>,
    mut graph_changed: MessageWriter<crate::GraphChanged>,
    mut node_graph_res: ResMut<crate::NodeGraphResource>,
    node_editor_settings: Res<crate::node_editor_settings::NodeEditorSettings>,
    q_tabs: Query<(&ShelfTabBtn, &cgui::Interaction), Changed<cgui::Interaction>>,
    q_add: Query<(&ShelfAddBtn, &cgui::Interaction), Changed<cgui::Interaction>>,
    q_new_set: Query<(&ShelfPopupItemNewSet, &cgui::Interaction), Changed<cgui::Interaction>>,
    q_toggle: Query<(&ShelfPopupItemToggleTab, &cgui::Interaction), Changed<cgui::Interaction>>,
    q_tool: Query<(&ShelfToolBtn, &cgui::Interaction), Changed<cgui::Interaction>>,
) {
    for (b, i) in &q_tabs {
        if *i == cgui::Interaction::Pressed {
            set_active_tab(&mut ui_state.shelf_dock_state, b.surface, b.node, b.idx);
            menu.open = None;
        }
    }
    for (b, i) in &q_add {
        if *i == cgui::Interaction::Pressed {
            let k = (b.surface, b.node);
            menu.open = if menu.open == Some(k) { None } else { Some(k) };
        }
    }
    for (b, i) in &q_new_set {
        if *i == cgui::Interaction::Pressed {
            ui_state
                .shelf_command_queue
                .push(ShelfCommand::NewSet(b.surface, b.node));
            menu.open = None;
        }
    }
    for (b, i) in &q_toggle {
        if *i == cgui::Interaction::Pressed {
            ui_state.shelf_command_queue.push(ShelfCommand::Toggle(
                b.tab.clone(),
                b.surface,
                b.node,
            ));
        }
    }
    for (b, i) in &q_tool {
        if *i != cgui::Interaction::Pressed {
            continue;
        }
        match b.0 {
            ShelfTool::Cube => { crate::ui::create_cube_node(
                &mut *ui_state,
                &mut *node_graph_res,
                &node_editor_settings,
                bevy_egui::egui::Pos2::ZERO,
            ); },
            ShelfTool::Sphere => { crate::ui::create_sphere_node(
                &mut *ui_state,
                &mut *node_graph_res,
                &node_editor_settings,
                bevy_egui::egui::Pos2::ZERO,
            ); },
            ShelfTool::Transform => { crate::ui::create_transform_node(
                &mut *ui_state,
                &mut *node_graph_res,
                &node_editor_settings,
                bevy_egui::egui::Pos2::ZERO,
            ); },
            ShelfTool::Promote => { crate::ui::create_attribute_promote_node(
                &mut *ui_state,
                &mut *node_graph_res,
                &node_editor_settings,
                bevy_egui::egui::Pos2::ZERO,
            ); },
            ShelfTool::Merge => { crate::ui::create_merge_node(
                &mut *ui_state,
                &mut *node_graph_res,
                &node_editor_settings,
                bevy_egui::egui::Pos2::ZERO,
            ); },
            ShelfTool::Dummy(_) => {}
        }
        graph_changed.write_default();
    }
}

fn close_popup_on_esc(keys: Res<ButtonInput<KeyCode>>, mut menu: ResMut<ShelfMenuState>) {
    if keys.just_pressed(KeyCode::Escape) {
        menu.open = None;
    }
}

fn apply_shelf_commands(mut ui_state: ResMut<UiState>) {
    let cmds: Vec<_> = ui_state.shelf_command_queue.drain(..).collect();
    for cmd in cmds {
        match cmd {
            ShelfCommand::Add(tab, surface, node) => {
                if let egui_dock::Node::Leaf { tabs, .. } =
                    &mut ui_state.shelf_dock_state[surface][node]
                {
                    tabs.push(tab);
                }
            }
            ShelfCommand::Remove(surface, node, index) => {
                ui_state.shelf_dock_state.remove_tab((surface, node, index));
            }
            ShelfCommand::Toggle(tab, surface, node) => {
                if let egui_dock::Node::Leaf { tabs, .. } =
                    &mut ui_state.shelf_dock_state[surface][node]
                {
                    if let Some(idx) = tabs.iter().position(|t| *t == tab) {
                        tabs.remove(idx);
                    } else {
                        tabs.push(tab);
                    }
                }
            }
            ShelfCommand::NewSet(surface, node) => {
                let _ = ui_state.shelf_dock_state[surface].split_right(
                    node,
                    0.5,
                    vec![ShelfTab::Create],
                );
            }
        }
    }
}
