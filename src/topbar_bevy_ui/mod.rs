//! Bevy UI Topbar (Desktop) - Replaces egui MenuBarTab
use bevy::camera::visibility::RenderLayers;
use bevy::math::CompassOctant;
use bevy::prelude::*;

use crate::console::ConsoleLog;
use crate::cunning_core::cda::CdaLibrary;
use crate::app::window_frame::{
    WINDOW_CHROME_BTN_H_LP, WINDOW_CHROME_BTN_W_LP, WINDOW_CHROME_GLYPH_CLOSE,
    WINDOW_CHROME_GLYPH_MAX, WINDOW_CHROME_GLYPH_MIN, WINDOW_CHROME_ICON_FONT_FAMILY,
    WINDOW_CORNER_RADIUS_LP, WINDOW_SAFE_INSET_LP, WINDOW_TOPBAR_H_LP,
    WINDOW_UI_SURFACE_BG_SRGBA,
};
use crate::launcher::plugin::AppState;
use crate::ui::{LayoutMode, OpenAiWorkspaceWindowEvent, OpenSettingsWindowEvent, UiState};
use crate::{GraphChanged, NodeGraphResource};
use bevy::ui::prelude as cgui;
use bevy::ui::{FocusPolicy, UiSystems};
use bevy::window::{CursorIcon, PrimaryWindow, RequestRedraw, SystemCursorIcon, WindowLevel};
use bevy_egui::EguiHole;

#[derive(Component)]
pub struct TopbarUiRoot;
#[derive(Component)]
struct TopbarBar;
#[derive(Component)]
struct TopbarMenuBtn(MenuKind);
#[derive(Component)]
struct TopbarAction(TopbarActionKind);
#[derive(Component)]
struct TopbarBackdrop;
#[derive(Component)]
struct TopbarPopupRoot;
#[derive(Component)]
struct TopbarPopupPanel;
#[derive(Component)]
struct TopbarInteractive;
#[derive(Component)]
struct TopbarDragRegion;
#[derive(Component)]
struct TopbarWindowBtn(WindowBtnKind);
#[derive(Component)]
struct TopbarPinLabel;
#[derive(Component)]
pub struct TopbarUiCamera;

/// Timeline camera reuse: let Timeline plugin reuse the same UI camera to avoid multi-camera overlay risks
use crate::timeline_bevy_ui::TimelineUiCamera;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MenuKind {
    File,
    View,
    Settings,
    Ai,
}

#[derive(Clone, Copy)]
enum TopbarActionKind {
    New,
    Open,
    Save,
    SaveAs,
    LoadCda,
    LayoutDesktop,
    LayoutTablet,
    LayoutPhone,
    OpenSettings,
    OpenAi,
}

#[derive(Resource, Default)]
struct TopbarMenuState {
    open: Option<MenuKind>,
}

/// Whether Topbar consumes input (for input gating)
#[derive(Resource, Default)]
pub struct TopbarUiWantsInput(pub bool);

/// Whether Topbar dropdown menu is open (used to let egui yield drawing/input in that area)
#[derive(Resource, Default)]
pub struct TopbarUiMenuOpen(pub bool);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WindowBtnKind {
    Pin,
    Min,
    Max,
    Close,
}

#[derive(Resource, Default)]
struct TopbarWindowChromeState {
    pinned: bool,
    maximized: bool,
    last_click_s: f64,
}

#[derive(Resource, Default)]
struct WindowEdgeResizeState {
    hover: Option<CompassOctant>,
}

/// Debug: automatically open one menu once to validate occlusion logging without relying on click.
#[derive(Resource, Default)]
struct TopbarDebugAutoOpen {
    enabled: bool,
    fired: bool,
}

pub struct TopbarUiPlugin;

const TOPBAR_H: f32 = WINDOW_TOPBAR_H_LP;
const UI_LAYER: usize = 31;
const WINDOW_CORNER_RADIUS_VISUAL: f32 = WINDOW_CORNER_RADIUS_LP;
const WINDOW_UI_SURFACE_COLOR: Color = Color::srgba(
    WINDOW_UI_SURFACE_BG_SRGBA[0],
    WINDOW_UI_SURFACE_BG_SRGBA[1],
    WINDOW_UI_SURFACE_BG_SRGBA[2],
    WINDOW_UI_SURFACE_BG_SRGBA[3],
);

#[derive(Resource, Default)]
struct TopbarUiSpawnGate {
    last: Option<UVec2>,
    same: u8,
    frames: u16,
    spawned: bool,
}

impl Plugin for TopbarUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TopbarMenuState>()
            .init_resource::<TopbarUiWantsInput>()
            .init_resource::<TopbarUiMenuOpen>()
            .init_resource::<TopbarUiSpawnGate>()
            .init_resource::<TopbarWindowChromeState>()
            .init_resource::<WindowEdgeResizeState>()
            .init_resource::<TopbarDebugAutoOpen>()
            .add_systems(OnEnter(AppState::Editor), reset_topbar_spawn_gate)
            // NOTE: Click/interaction relies on ui_focus_system (PreUpdate/UiSystems::Focus) to write Interaction;
            // These handlers must run after it, otherwise "highlighted but not clickable/no dropdown" phenomenon will occur.
            .add_systems(
                PreUpdate,
                (
                    handle_topbar_buttons,
                    handle_topbar_drag_region,
                    handle_topbar_window_buttons,
                    #[cfg(debug_assertions)]
                    debug_auto_open_menu_once,
                    close_menu_on_backdrop,
                    handle_topbar_actions,
                    close_menu_on_esc,
                )
                    .after(UiSystems::Focus)
                    .run_if(in_state(AppState::Editor)),
            )
            .add_systems(
                Update,
                (
                    spawn_topbar_ui_when_window_stable,
                    sync_topbar_visibility,
                    // Critical: popup must be generated after menu state update, otherwise it will "look unclickable" in reactive mode
                    sync_topbar_popup,
                    topbar_visuals,
                    update_topbar_wants_input.after(sync_topbar_popup),
                    window_edge_resize_system,
                )
                    .run_if(in_state(AppState::Editor)),
            );
    }
}

fn reset_topbar_spawn_gate(mut g: ResMut<TopbarUiSpawnGate>) {
    *g = default();
}

#[cfg(debug_assertions)]
fn debug_auto_open_menu_once(
    mut dbg: ResMut<TopbarDebugAutoOpen>,
    ui_state: Res<UiState>,
    popup_q: Query<Entity, With<TopbarPopupRoot>>,
    mut menu: ResMut<TopbarMenuState>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    // Enable by env: CGUI_DEBUG_AUTO_OPEN=1
    if !dbg.enabled {
        dbg.enabled = std::env::var("CGUI_DEBUG_AUTO_OPEN").ok().as_deref() == Some("1");
        if !dbg.enabled {
            return;
        }
    }
    if dbg.fired {
        return;
    }
    if !matches!(ui_state.layout_mode, LayoutMode::Desktop) {
        return;
    }
    let root_count = popup_q.iter().count();
    if root_count == 0 {
        return;
    }
    if menu.open.is_none() {
        menu.open = Some(MenuKind::File);
        dbg.fired = true;
        redraw.write(RequestRedraw);
        bevy::log::warn!(
            "TOPBAR_OCCLUSION debug auto-open triggered (popup_roots={})",
            root_count
        );
    }
}

fn spawn_topbar_ui_when_window_stable(
    mut commands: Commands,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut g: ResMut<TopbarUiSpawnGate>,
    cam_q: Query<Entity, With<TopbarUiCamera>>,
    timeline_cam_q: Query<Entity, With<TimelineUiCamera>>,
) {
    if g.spawned || !cam_q.is_empty() {
        return;
    }
    let Ok(w) = windows.single() else {
        return;
    };
    let size = UVec2::new(w.physical_width(), w.physical_height());
    if size.x == 0 || size.y == 0 {
        return;
    }
    g.frames = g.frames.saturating_add(1);
    if g.last == Some(size) {
        g.same = g.same.saturating_add(1);
    } else {
        g.last = Some(size);
        g.same = 0;
    }
    let big_enough = size.x >= 600 && size.y >= 400;
    let stable_enough = g.same >= 1;
    let timeout = g.frames >= 60;
    if !(big_enough && (stable_enough || timeout)) {
        return;
    }
    g.spawned = true;
    // IMPORTANT: never spawn a second UI camera (will trigger camera order ambiguities and unpredictable rendering).
    // We always reuse the Timeline UI camera.
    if timeline_cam_q.is_empty() {
        g.spawned = false;
        return;
    }
    spawn_topbar_ui(&mut commands, &timeline_cam_q);
    info!(
        "Spawn Topbar UI (bevy_ui) at physical window size: {size:?}, same_frames={}, frames={}",
        g.same, g.frames
    );
}

fn spawn_topbar_ui(
    commands: &mut Commands,
    timeline_cam_q: &Query<Entity, With<TimelineUiCamera>>,
) {
    let Some(cam) = timeline_cam_q.iter().next() else {
        return;
    };
    commands
        .entity(cam)
        .try_insert((TopbarUiCamera, RenderLayers::layer(UI_LAYER)));

    // Fullscreen root: absolute overlay + explicit (Global)ZIndex to avoid any stack/clip surprises.
    let root = commands
        .spawn_empty()
        .insert(TopbarUiRoot)
        .insert(cgui::UiTargetCamera(cam))
        .insert(RenderLayers::layer(UI_LAYER))
        .insert(cgui::GlobalZIndex(9))
        .insert(cgui::ZIndex(0))
        .insert(cgui::Node {
            position_type: cgui::PositionType::Absolute,
            left: cgui::Val::Px(0.0),
            right: cgui::Val::Px(0.0),
            top: cgui::Val::Px(0.0),
            bottom: cgui::Val::Px(0.0),
            ..default()
        })
        .id();

    // Top safe inset strip: keep content off the very top edge.
    let top_strip = commands
        .spawn_empty()
        .insert(RenderLayers::layer(UI_LAYER))
        .insert(cgui::GlobalZIndex(19))
        .insert(cgui::ZIndex(0))
        // Extend drag region into the top safe inset strip (easier window move at y=0).
        .insert(TopbarDragRegion)
        .insert(cgui::Button)
        .insert(cgui::Interaction::None)
        .insert(FocusPolicy::Pass)
        .insert(cgui::Node {
            position_type: cgui::PositionType::Absolute,
            left: cgui::Val::Px(0.0),
            right: cgui::Val::Px(0.0),
            top: cgui::Val::Px(0.0),
            height: cgui::Val::Px(WINDOW_SAFE_INSET_LP),
            ..default()
        })
        .insert(cgui::BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.0)))
        .id();
    commands.entity(root).add_child(top_strip);

    let bar = commands
        .spawn_empty()
        .insert(TopbarBar)
        .insert(EguiHole)
        .insert(RenderLayers::layer(UI_LAYER))
        .insert(cgui::Node {
            position_type: cgui::PositionType::Absolute,
            left: cgui::Val::Px(0.0),
            right: cgui::Val::Px(0.0),
            top: cgui::Val::Px(0.0),
            height: cgui::Val::Px(WINDOW_SAFE_INSET_LP + TOPBAR_H),
            flex_direction: cgui::FlexDirection::Row,
            align_items: cgui::AlignItems::Center,
            // Full-width rounded top bar; content remains inside safe inset.
            padding: cgui::UiRect {
                left: cgui::Val::Px(WINDOW_SAFE_INSET_LP + 12.0),
                right: cgui::Val::Px(WINDOW_SAFE_INSET_LP + 12.0),
                top: cgui::Val::Px(WINDOW_SAFE_INSET_LP),
                bottom: cgui::Val::Px(0.0),
            },
            column_gap: cgui::Val::Px(6.0),
            border_radius: cgui::BorderRadius::new(
                cgui::Val::Px(WINDOW_CORNER_RADIUS_VISUAL),
                cgui::Val::Px(WINDOW_CORNER_RADIUS_VISUAL),
                cgui::Val::Px(0.0),
                cgui::Val::Px(0.0),
            ),
            ..default()
        })
        .insert(cgui::BackgroundColor(WINDOW_UI_SURFACE_COLOR))
        .id();
    commands.entity(root).add_child(bar);

    for (kind, label) in [
        (MenuKind::File, "File"),
        (MenuKind::View, "View"),
        (MenuKind::Settings, "Settings"),
        (MenuKind::Ai, "AI"),
    ] {
        let btn = commands
            .spawn_empty()
            .insert(TopbarMenuBtn(kind))
            .insert(RenderLayers::layer(UI_LAYER))
            .insert(cgui::Button)
            .insert(cgui::Interaction::None)
            .insert(TopbarInteractive)
            .insert(cgui::Node {
                width: cgui::Val::Auto,
                height: cgui::Val::Px(TOPBAR_H - 6.0),
                padding: cgui::UiRect::horizontal(cgui::Val::Px(10.0)),
                justify_content: cgui::JustifyContent::Center,
                align_items: cgui::AlignItems::Center,
                border_radius: cgui::BorderRadius::all(cgui::Val::Px(4.0)),
                ..default()
            })
            .insert(cgui::BackgroundColor(Color::srgba(0.18, 0.18, 0.18, 1.0)))
            .id();
        let txt = commands
            .spawn_empty()
            .insert(RenderLayers::layer(UI_LAYER))
            .insert(cgui::Text::new(label))
            .insert(TextFont {
                font_size: FontSize::Px(12.0),
                ..default()
            })
            .insert(TextColor(Color::srgba(0.92, 0.92, 0.92, 1.0)))
            .id();
        commands.entity(btn).add_child(txt);
        commands.entity(bar).add_child(btn);
    }

    // Drag region (fills remaining space; used for window move + double-click maximize)
    let drag = commands
        .spawn_empty()
        .insert(TopbarDragRegion)
        .insert(RenderLayers::layer(UI_LAYER))
        .insert(cgui::Button)
        .insert(cgui::Interaction::None)
        .insert(FocusPolicy::Pass)
        .insert(cgui::Node {
            flex_grow: 1.0,
            height: cgui::Val::Px(TOPBAR_H - 6.0),
            ..default()
        })
        .insert(cgui::BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.0)))
        .id();
    commands.entity(bar).add_child(drag);

    // Window controls (right side)
    let controls = commands
        .spawn_empty()
        .insert(RenderLayers::layer(UI_LAYER))
        .insert(cgui::Node {
            flex_direction: cgui::FlexDirection::Row,
            align_items: cgui::AlignItems::Center,
            column_gap: cgui::Val::Px(4.0),
            ..default()
        })
        .id();
    commands.entity(bar).add_child(controls);

    let mk_btn = |id: WindowBtnKind, label: &str, commands: &mut Commands, controls: Entity| {
        let b = commands
            .spawn_empty()
            .insert(TopbarWindowBtn(id))
            .insert(RenderLayers::layer(UI_LAYER))
            .insert(cgui::Button)
            .insert(cgui::Interaction::None)
            .insert(TopbarInteractive)
            .insert(cgui::Node {
                width: cgui::Val::Px(WINDOW_CHROME_BTN_W_LP),
                height: cgui::Val::Px(WINDOW_CHROME_BTN_H_LP),
                justify_content: cgui::JustifyContent::Center,
                align_items: cgui::AlignItems::Center,
                border_radius: cgui::BorderRadius::all(cgui::Val::Px(4.0)),
                ..default()
            })
            .insert(cgui::BackgroundColor(Color::srgba(0.18, 0.18, 0.18, 1.0)))
            .id();
        let t = commands
            .spawn_empty()
            .insert(RenderLayers::layer(UI_LAYER))
            .insert(cgui::Text::new(label))
            .insert(TextFont { font_size: FontSize::Px(11.0), ..default() })
            .insert(TextColor(Color::srgba(0.92, 0.92, 0.92, 1.0)))
            .id();
        commands.entity(b).add_child(t);
        commands.entity(controls).add_child(b);
        (b, t)
    };
    let (_pin_btn, pin_txt) = mk_btn(WindowBtnKind::Pin, "📍", commands, controls);
    commands.entity(pin_txt).insert(TopbarPinLabel);
    let (_min_btn, min_txt) = mk_btn(WindowBtnKind::Min, WINDOW_CHROME_GLYPH_MIN, commands, controls);
    commands.entity(min_txt).insert(TextFont { font_size: FontSize::Px(11.0), font: WINDOW_CHROME_ICON_FONT_FAMILY.into(), ..default() });
    let (_max_btn, max_txt) = mk_btn(WindowBtnKind::Max, WINDOW_CHROME_GLYPH_MAX, commands, controls);
    commands.entity(max_txt).insert(TextFont { font_size: FontSize::Px(11.0), font: WINDOW_CHROME_ICON_FONT_FAMILY.into(), ..default() });
    let (close_btn, close_txt) = mk_btn(WindowBtnKind::Close, WINDOW_CHROME_GLYPH_CLOSE, commands, controls);
    commands.entity(close_txt).insert(TextFont { font_size: FontSize::Px(11.0), font: WINDOW_CHROME_ICON_FONT_FAMILY.into(), ..default() });
    // Close button: subtle danger tint.
    commands
        .entity(close_btn)
        .insert(cgui::BackgroundColor(Color::srgba(0.30, 0.10, 0.10, 1.0)));

    // popup root (initially empty)
    let popup = commands
        .spawn_empty()
        .insert(TopbarPopupRoot)
        .insert(RenderLayers::layer(UI_LAYER))
        .insert(cgui::Interaction::None)
        .insert(FocusPolicy::Pass)
        .insert(cgui::GlobalZIndex(10))
        .insert(cgui::ZIndex(0))
        .insert(cgui::Node {
            position_type: cgui::PositionType::Absolute,
            left: cgui::Val::Px(0.0),
            right: cgui::Val::Px(0.0),
            top: cgui::Val::Px(WINDOW_SAFE_INSET_LP + TOPBAR_H),
            bottom: cgui::Val::Px(0.0),
            ..default()
        })
        .id();
    commands.entity(root).add_child(popup);
}

fn sync_topbar_visibility(
    ui_state: Res<UiState>,
    mut q: Query<&mut cgui::Node, With<TopbarUiRoot>>,
) {
    for mut n in &mut q {
        let _ = ui_state;
        n.display = cgui::Display::Flex;
    }
}

fn handle_topbar_buttons(
    ui_state: Res<UiState>,
    mut menu: ResMut<TopbarMenuState>,
    btn_q: Query<(&TopbarMenuBtn, &Interaction), Changed<Interaction>>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let _ = ui_state;
    for (b, i) in &btn_q {
        if *i != Interaction::Pressed {
            continue;
        }
        menu.open = if menu.open == Some(b.0) {
            None
        } else {
            Some(b.0)
        };
        redraw.write(RequestRedraw);
    }
}

fn topbar_visuals(
    mut q: Query<
        (
            &Interaction,
            &mut BackgroundColor,
            Option<&TopbarMenuBtn>,
            Option<&TopbarAction>,
            Option<&TopbarWindowBtn>,
            Option<&TopbarBackdrop>,
        ),
        (Changed<Interaction>, With<TopbarInteractive>),
    >,
) {
    for (i, mut bg, is_menu, is_act, is_win, is_backdrop) in &mut q {
        if is_backdrop.is_some() {
            continue;
        }
        let base = match (is_menu.is_some(), is_act.is_some(), is_win.map(|w| w.0)) {
            (_, _, Some(WindowBtnKind::Close)) => Color::srgba(0.30, 0.10, 0.10, 1.0),
            _ => Color::srgba(0.18, 0.18, 0.18, 1.0),
        };
        *bg = match *i {
            Interaction::None => BackgroundColor(base),
            Interaction::Hovered => BackgroundColor(if matches!(is_win.map(|w| w.0), Some(WindowBtnKind::Close)) { Color::srgba(0.38, 0.14, 0.14, 1.0) } else { Color::srgba(0.26, 0.26, 0.26, 1.0) }),
            Interaction::Pressed => BackgroundColor(if matches!(is_win.map(|w| w.0), Some(WindowBtnKind::Close)) { Color::srgba(0.48, 0.18, 0.18, 1.0) } else { Color::srgba(0.36, 0.36, 0.36, 1.0) }),
        };
    }
}

fn handle_topbar_drag_region(
    mut chrome: ResMut<TopbarWindowChromeState>,
    time: Res<Time>,
    mut w_q: Query<&mut Window, With<PrimaryWindow>>,
    q: Query<&Interaction, (With<TopbarDragRegion>, Changed<Interaction>)>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let Ok(mut w) = w_q.single_mut() else { return; };
    for i in &q {
        if *i != Interaction::Pressed {
            continue;
        }
        let now = time.elapsed_secs_f64();
        let dbl = (now - chrome.last_click_s) <= 0.35;
        chrome.last_click_s = now;
        if dbl {
            chrome.maximized = !chrome.maximized;
            w.set_maximized(chrome.maximized);
        } else {
            w.start_drag_move();
        }
        redraw.write(RequestRedraw);
    }
}

fn handle_topbar_window_buttons(
    mut chrome: ResMut<TopbarWindowChromeState>,
    mut w_q: Query<&mut Window, With<PrimaryWindow>>,
    mut exit: MessageWriter<bevy::app::AppExit>,
    q: Query<(&TopbarWindowBtn, &Interaction), Changed<Interaction>>,
    mut pin_txt: Query<&mut cgui::Text, With<TopbarPinLabel>>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let Ok(mut w) = w_q.single_mut() else { return; };
    for (b, i) in &q {
        if *i != Interaction::Pressed {
            continue;
        }
        match b.0 {
            WindowBtnKind::Pin => {
                chrome.pinned = !chrome.pinned;
                w.window_level = if chrome.pinned { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal };
                for mut t in &mut pin_txt {
                    *t = cgui::Text::new(if chrome.pinned { "📌" } else { "📍" });
                }
            }
            WindowBtnKind::Min => w.set_minimized(true),
            WindowBtnKind::Max => {
                chrome.maximized = !chrome.maximized;
                w.set_maximized(chrome.maximized);
            }
            WindowBtnKind::Close => { exit.write(bevy::app::AppExit::Success); }
        }
        redraw.write(RequestRedraw);
    }
}

fn window_edge_resize_system(
    mut commands: Commands,
    mut st: ResMut<WindowEdgeResizeState>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut w_q: Query<(Entity, &mut Window), With<PrimaryWindow>>,
) {
    let Ok((e, mut w)) = w_q.single_mut() else { return; };
    if w.decorations {
        st.hover = None;
        return;
    }
    let Some(p) = w.cursor_position() else {
        st.hover = None;
        return;
    };
    let s = w.size();
    let b = 6.0;
    let l = p.x <= b;
    let r = p.x >= s.x - b;
    let t = p.y <= b;
    let bt = p.y >= s.y - b;
    let dir = match (l, r, t, bt) {
        (true, _, true, _) => Some(CompassOctant::NorthWest),
        (_, true, true, _) => Some(CompassOctant::NorthEast),
        (true, _, _, true) => Some(CompassOctant::SouthWest),
        (_, true, _, true) => Some(CompassOctant::SouthEast),
        (_, _, true, _) => Some(CompassOctant::North),
        (_, _, _, true) => Some(CompassOctant::South),
        (true, _, _, _) => Some(CompassOctant::West),
        (_, true, _, _) => Some(CompassOctant::East),
        _ => None,
    };
    if dir != st.hover {
        st.hover = dir;
        if let Some(d) = dir {
            let cur = match d {
                CompassOctant::North => SystemCursorIcon::NResize,
                CompassOctant::NorthEast => SystemCursorIcon::NeResize,
                CompassOctant::East => SystemCursorIcon::EResize,
                CompassOctant::SouthEast => SystemCursorIcon::SeResize,
                CompassOctant::South => SystemCursorIcon::SResize,
                CompassOctant::SouthWest => SystemCursorIcon::SwResize,
                CompassOctant::West => SystemCursorIcon::WResize,
                CompassOctant::NorthWest => SystemCursorIcon::NwResize,
            };
            commands.entity(e).insert(CursorIcon::from(cur));
        } else {
            commands.entity(e).insert(CursorIcon::from(SystemCursorIcon::Default));
        }
    }
    if mouse.just_pressed(MouseButton::Left) {
        if let Some(d) = dir {
            w.start_drag_resize(d);
        }
    }
}

fn sync_topbar_popup(
    mut commands: Commands,
    ui_state: Res<UiState>,
    menu: Res<TopbarMenuState>,
    popup_q: Query<Entity, With<TopbarPopupRoot>>,
    children_q: Query<&Children>,
    mut last: Local<(LayoutMode, Option<MenuKind>)>,
) {
    // Only rebuild when (layout_mode, open_menu) changes; otherwise keep the popup entities stable.
    let cur = (ui_state.layout_mode, menu.open);
    if *last == cur {
        return;
    }
    *last = cur;
    let Some(popup_e) = popup_q.iter().next() else {
        return;
    };
    // Clear old popup (including backdrop)
    // NOTE: Bevy 0.18 `despawn()` already recursively despawns relationship descendants (e.g. Children).
    // Manual recursion here can cause double-despawn command errors + hitching.
    if let Ok(kids) = children_q.get(popup_e) {
        for &c in kids {
            commands.entity(c).despawn();
        }
    }
    let Some(kind) = menu.open else {
        return;
    };
    // Backdrop captures outside click
    let _backdrop = commands
        .spawn_empty()
        .insert(TopbarBackdrop)
        .insert(RenderLayers::layer(UI_LAYER))
        .insert(cgui::Button)
        .insert(cgui::Interaction::None)
        .insert(TopbarInteractive)
        .insert(cgui::Node {
            position_type: cgui::PositionType::Absolute,
            left: cgui::Val::Px(0.0),
            right: cgui::Val::Px(0.0),
            top: cgui::Val::Px(0.0),
            bottom: cgui::Val::Px(0.0),
            ..default()
        })
        .insert(cgui::BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.0)))
        .id();
    commands.entity(popup_e).add_child(_backdrop);

    let panel = commands
        .spawn_empty()
        .insert(TopbarPopupPanel)
        .insert(EguiHole)
        .insert(RenderLayers::layer(UI_LAYER))
        .insert(cgui::Interaction::None)
        .insert(cgui::Node {
            position_type: cgui::PositionType::Absolute,
            left: cgui::Val::Px(8.0),
            top: cgui::Val::Px(2.0),
            width: cgui::Val::Px(180.0),
            flex_direction: cgui::FlexDirection::Column,
            padding: cgui::UiRect::all(cgui::Val::Px(6.0)),
            row_gap: cgui::Val::Px(2.0),
            border_radius: cgui::BorderRadius::all(cgui::Val::Px(6.0)),
            ..default()
        })
        .insert(cgui::BackgroundColor(Color::srgba(0.13, 0.13, 0.13, 0.98)))
        .id();
    commands.entity(popup_e).add_child(panel);

    let add_item = |label: &str, act: TopbarActionKind, commands: &mut Commands, panel: Entity| {
        let b = commands
            .spawn_empty()
            .insert(TopbarAction(act))
            .insert(RenderLayers::layer(UI_LAYER))
            .insert(cgui::Button)
            .insert(cgui::Interaction::None)
            .insert(TopbarInteractive)
            .insert(cgui::Node {
                height: cgui::Val::Px(22.0),
                padding: cgui::UiRect::horizontal(cgui::Val::Px(8.0)),
                align_items: cgui::AlignItems::Center,
                border_radius: cgui::BorderRadius::all(cgui::Val::Px(4.0)),
                ..default()
            })
            .insert(cgui::BackgroundColor(Color::srgba(0.18, 0.18, 0.18, 1.0)))
            .id();
        let t = commands
            .spawn_empty()
            .insert(RenderLayers::layer(UI_LAYER))
            .insert(cgui::Text::new(label))
            .insert(TextFont {
                font_size: FontSize::Px(12.0),
                ..default()
            })
            .insert(TextColor(Color::srgba(0.92, 0.92, 0.92, 1.0)))
            .id();
        commands.entity(b).add_child(t);
        commands.entity(panel).add_child(b);
    };

    match kind {
        MenuKind::File => {
            add_item("New", TopbarActionKind::New, &mut commands, panel);
            add_item("Open...", TopbarActionKind::Open, &mut commands, panel);
            add_item("Save", TopbarActionKind::Save, &mut commands, panel);
            add_item("Save As...", TopbarActionKind::SaveAs, &mut commands, panel);
            add_item(
                "Load CDA...",
                TopbarActionKind::LoadCda,
                &mut commands,
                panel,
            );
        }
        MenuKind::View => {
            add_item(
                "Layout: Desktop",
                TopbarActionKind::LayoutDesktop,
                &mut commands,
                panel,
            );
            add_item(
                "Layout: Tablet",
                TopbarActionKind::LayoutTablet,
                &mut commands,
                panel,
            );
            add_item(
                "Layout: Phone",
                TopbarActionKind::LayoutPhone,
                &mut commands,
                panel,
            );
        }
        MenuKind::Settings => {
            add_item("Open Settings", TopbarActionKind::OpenSettings, &mut commands, panel);
        }
        MenuKind::Ai => add_item(
            "Open AI Workspace",
            TopbarActionKind::OpenAi,
            &mut commands,
            panel,
        ),
    }
}

fn close_menu_on_backdrop(
    ui_state: Res<UiState>,
    mut menu: ResMut<TopbarMenuState>,
    q: Query<&cgui::Interaction, (With<TopbarBackdrop>, Changed<cgui::Interaction>)>,
) {
    let _ = ui_state;
    if menu.open.is_none() {
        return;
    }
    if q.iter().any(|i| *i == cgui::Interaction::Pressed) {
        menu.open = None;
    }
}

fn handle_topbar_actions(
    mut menu: ResMut<TopbarMenuState>,
    mut graph_changed: MessageWriter<GraphChanged>,
    mut open_settings: MessageWriter<OpenSettingsWindowEvent>,
    mut open_ai: MessageWriter<OpenAiWorkspaceWindowEvent>,
    mut open_hr: MessageWriter<crate::ui::OpenHotReloadWindowEvent>,
    mut open_file_picker: MessageWriter<crate::ui::OpenFilePickerEvent>,
    mut redraw: MessageWriter<RequestRedraw>,
    mut ui_state_mut: ResMut<UiState>,
    mut node_graph_res: ResMut<NodeGraphResource>,
    time: Res<Time>,
    cda_lib: Res<CdaLibrary>,
    console: Res<ConsoleLog>,
    hot_log: Option<Res<crate::tabs_system::pane::hot_reload::HotReloadLog>>,
    ps: Option<Res<crate::cunning_core::plugin_system::PluginSystem>>,
    reg: Option<Res<crate::cunning_core::registries::node_registry::NodeRegistry>>,
    act_q: Query<(&TopbarAction, &cgui::Interaction), Changed<cgui::Interaction>>,
) {
    for (a, i) in &act_q {
        if *i != cgui::Interaction::Pressed {
            continue;
        }
        match a.0 {
            TopbarActionKind::New => {
                node_graph_res.0 = crate::nodes::NodeGraph::default();
                graph_changed.write_default();
            }
            TopbarActionKind::Open => {
                open_file_picker.write(crate::ui::OpenFilePickerEvent {
                    mode: crate::ui::FilePickerMode::OpenProject,
                });
            }
            TopbarActionKind::Save | TopbarActionKind::SaveAs => {
                open_file_picker.write(crate::ui::OpenFilePickerEvent {
                    mode: if matches!(a.0, TopbarActionKind::SaveAs) {
                        crate::ui::FilePickerMode::SaveProjectAs
                    } else {
                        crate::ui::FilePickerMode::SaveProject
                    },
                });
            }
            TopbarActionKind::LoadCda => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("CDA", &["cda"])
                    .pick_file()
                {
                    match crate::cunning_core::cda::CDAAsset::load_dcc(&path) {
                        Ok(asset) => {
                            cda_lib
                                .put_with_path(asset.clone(), path.to_string_lossy().to_string());
                            console.info(format!("Loaded CDA: {} ({})", asset.name, asset.id));
                            bevy::log::info!("Loaded CDA: {} ({})", asset.name, asset.id);
                        }
                        Err(e) => {
                            let msg =
                                format!("Load CDA failed: {:?} ({})", e, path.to_string_lossy());
                            console.error(msg.clone());
                            bevy::log::warn!("{}", msg);
                        }
                    }
                }
            }
            TopbarActionKind::LayoutDesktop => ui_state_mut.layout_mode = LayoutMode::Desktop,
            TopbarActionKind::LayoutTablet => ui_state_mut.layout_mode = LayoutMode::Tablet,
            TopbarActionKind::LayoutPhone => ui_state_mut.layout_mode = LayoutMode::Phone,
            TopbarActionKind::OpenSettings => { open_settings.write_default(); }
            TopbarActionKind::OpenAi => { open_ai.write_default(); }
        }
        menu.open = None;
        redraw.write(RequestRedraw);
    }
}

fn close_menu_on_esc(
    keys: Res<ButtonInput<KeyCode>>,
    ui_state: Res<UiState>,
    mut menu: ResMut<TopbarMenuState>,
) {
    let _ = ui_state;
    if keys.just_pressed(KeyCode::Escape) {
        menu.open = None;
    }
}

fn update_topbar_wants_input(
    ui_state: Res<UiState>,
    menu: Res<TopbarMenuState>,
    q: Query<(&cgui::Interaction, Option<&TopbarBackdrop>), With<TopbarInteractive>>,
    mut wants: ResMut<TopbarUiWantsInput>,
    mut open: ResMut<TopbarUiMenuOpen>,
) {
    let _ = ui_state;
    open.0 = menu.open.is_some();
    // IMPORTANT: do NOT lock the 3D viewport just because the menu is open / backdrop is hovered.
    // Only claim input on actual presses on topbar UI (buttons / menu items).
    wants.0 = q
        .iter()
        .any(|(i, is_backdrop)| is_backdrop.is_none() && *i == cgui::Interaction::Pressed);
}
