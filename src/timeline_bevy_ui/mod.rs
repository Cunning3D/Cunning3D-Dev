//! Bevy UI Timeline - retained-mode timeline implementation
use crate::launcher::plugin::AppState;
use crate::ui::TimelineState;
use crate::app::window_frame::{
    WINDOW_CORNER_RADIUS_LP, WINDOW_SAFE_INSET_LP, WINDOW_UI_SURFACE_BG_SRGBA,
};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::ClearColorConfig;
use bevy::camera::{CameraOutputMode, MsaaWriteback};
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;
use bevy_cgui::{ComputedNode, FocusPolicy, UiSystems};
use bevy_cgui_widgets::{Slider, SliderRange, SliderValue, ValueChange};
use bevy_egui::EguiHole;
use bevy_input_focus::InputFocus;

pub mod icons;
pub mod numeric_input;
use icons::{spawn_icon, IconKind};
use numeric_input::{NumericInput, NumericInputPlugin, NumericInputValue};

/// Marker for the Timeline UI root entity
#[derive(Component)]
pub struct TimelineUiRoot;

/// Dedicated camera for Timeline UI (prevents UI from being clipped by the 3D viewport's Camera.viewport)
#[derive(Component)]
pub struct TimelineUiCamera;

const TIMELINE_UI_LAYER: usize = 31;

/// Playback button type
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimelineButton {
    First,
    Prev,
    PlayPause,
    Next,
    Last,
}

/// Track component
#[derive(Component)]
pub struct TimelineTrack;

#[derive(Component)]
struct TimelineTicksRoot;

#[derive(Component)]
struct TimelinePlayhead;

/// Numeric input type
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimelineField {
    Current,
    Start,
    End,
    Fps,
}

/// Marker for the PlayPause button icon container
#[derive(Component)]
struct PlayPauseIconContainer(bool); // true = currently showing the Play icon

/// Whether the Timeline UI consumes input (for input gating)
#[derive(Resource, Default)]
pub struct TimelineUiWantsInput(pub bool);

pub struct TimelineUiPlugin;

#[derive(Resource, Default)]
struct TimelineUiSpawnGate {
    last: Option<UVec2>,
    same: u8,
    frames: u16,
    spawned: bool,
}

#[derive(Resource, Default, Clone, Copy)]
struct TimelineTickCache {
    start: u32,
    end: u32,
    step: u32,
    minor: u32,
    track_w: u32,
}

impl Plugin for TimelineUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(NumericInputPlugin)
            .init_resource::<TimelineUiWantsInput>()
            .init_resource::<TimelineUiSpawnGate>()
            .init_resource::<TimelineTickCache>()
            .add_systems(OnEnter(AppState::Editor), reset_timeline_spawn_gate)
            .add_systems(
                Update,
                (
                    spawn_timeline_ui_when_window_stable,
                    sync_timeline_state_to_ui,
                    handle_button_clicks,
                    update_timeline_wants_input,
                )
                    .run_if(in_state(AppState::Editor)),
            )
            .add_systems(
                PostUpdate,
                update_timeline_track_decorations
                    .after(UiSystems::Layout)
                    .run_if(in_state(AppState::Editor)),
            )
            .add_observer(handle_track_value_change)
            .add_observer(handle_field_value_change);
    }
}

fn reset_timeline_spawn_gate(mut g: ResMut<TimelineUiSpawnGate>) {
    *g = default();
}

fn spawn_timeline_ui_when_window_stable(
    mut commands: Commands,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut g: ResMut<TimelineUiSpawnGate>,
    cam_q: Query<Entity, With<TimelineUiCamera>>,
) {
    if g.spawned {
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
    // Key: avoid a transition frame from splash (420x260) -> editor (1280x800); window maximize / multi-monitor DPI can still cause size jitter.
    // Relaxed strategy: if the size is clearly larger than the splash, require only 1 stable frame; otherwise wait up to ~1s then force spawn.
    let big_enough = size.x >= 600 && size.y >= 400;
    let stable_enough = g.same >= 1;
    let timeout = g.frames >= 60;
    if !(big_enough && (stable_enough || timeout)) {
        return;
    }
    g.spawned = true;
    info!(
        "Spawn Timeline UI (bevy_ui) at physical window size: {size:?}, same_frames={}, frames={}",
        g.same, g.frames
    );
    spawn_timeline_ui(&mut commands, &cam_q);
}

fn spawn_timeline_ui(commands: &mut Commands, cam_q: &Query<Entity, With<TimelineUiCamera>>) {
    // Note: avoid spawning an extra Camera3d here (it would trigger PBR/Prepass pipeline compilation; this project can hit a wgpu fatal).
    // CorePipelinePlugin already includes Core2dPlugin, so Camera2d is available and sufficient for the bevy_ui_render UI pass.
    let cam = cam_q.iter().next().unwrap_or_else(|| {
        commands
            .spawn((
                TimelineUiCamera,
                RenderLayers::layer(TIMELINE_UI_LAYER),
                Camera2d,
                Camera {
                    order: 100,
                    viewport: None,
                    // Key: clear with transparent color so \"undrawn regions have alpha=0\"; otherwise the whole screen gets written black.
                    clear_color: ClearColorConfig::Custom(Color::srgba(0.0, 0.0, 0.0, 0.0)),
                    // Key: use alpha blending when writing back to the main window (composited over the previous camera output).
                    output_mode: CameraOutputMode::Write {
                        blend_state: Some(BlendState::ALPHA_BLENDING),
                        clear_color: ClearColorConfig::None,
                    },
                    // Key: preserve the previous camera output with multi-camera + MSAA.
                    msaa_writeback: MsaaWriteback::Auto,
                    ..default()
                },
            ))
            .id()
    });

    let root = commands
        .spawn_empty()
        .insert(TimelineUiRoot)
        .insert(EguiHole)
        .insert(UiTargetCamera(cam))
        .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
        .insert(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(0.0),
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            height: Val::Px(60.0 + WINDOW_SAFE_INSET_LP),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            // Full-width background, but keep contents aligned with egui safe inset.
            padding: UiRect {
                left: Val::Px(WINDOW_SAFE_INSET_LP + 12.0),
                right: Val::Px(WINDOW_SAFE_INSET_LP + 12.0),
                top: Val::Px(4.0),
                bottom: Val::Px(WINDOW_SAFE_INSET_LP + 4.0),
            },
            column_gap: Val::Px(8.0),
            border_radius: BorderRadius::new(
                Val::Px(0.0),
                Val::Px(0.0),
                Val::Px(WINDOW_CORNER_RADIUS_LP),
                Val::Px(WINDOW_CORNER_RADIUS_LP),
            ),
            ..default()
        })
        .insert(BackgroundColor(Color::srgba(
            WINDOW_UI_SURFACE_BG_SRGBA[0],
            WINDOW_UI_SURFACE_BG_SRGBA[1],
            WINDOW_UI_SURFACE_BG_SRGBA[2],
            WINDOW_UI_SURFACE_BG_SRGBA[3],
        )))
        .insert(Interaction::None)
        .insert(FocusPolicy::Pass)
        .id();

    // Left: playback control button container
    let btn_container = commands
        .spawn_empty()
        .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
        .insert(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(2.0),
            ..default()
        })
        .id();
    commands.entity(root).add_child(btn_container);

    // Buttons (icon-based)
    for btn in [
        TimelineButton::First,
        TimelineButton::Prev,
        TimelineButton::PlayPause,
        TimelineButton::Next,
        TimelineButton::Last,
    ] {
        let btn_id = commands
            .spawn_empty()
            .insert(btn)
            .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
            .insert(Node {
                width: Val::Px(28.0),
                height: Val::Px(28.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..default()
            })
            .insert(BackgroundColor(Color::srgba(0.25, 0.25, 0.25, 1.0)))
            .insert(Interaction::None)
            .id();
        let icon_kind = match btn {
            TimelineButton::First => IconKind::First,
            TimelineButton::Prev => IconKind::Prev,
            TimelineButton::PlayPause => IconKind::Play,
            TimelineButton::Next => IconKind::Next,
            TimelineButton::Last => IconKind::Last,
        };
        let icon_e = spawn_icon(commands, icon_kind, 20.0);
        if btn == TimelineButton::PlayPause {
            commands.entity(icon_e).insert(PlayPauseIconContainer(true));
        }
        commands.entity(btn_id).add_child(icon_e);
        commands.entity(btn_container).add_child(btn_id);
    }

    // Current frame numeric input
    let current_field = spawn_field_entity(commands, TimelineField::Current, 1.0, 1.0, 9999.0);
    commands.entity(root).add_child(current_field);

    // Tracks
    let track = commands
        .spawn_empty()
        .insert(TimelineTrack)
        .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
        .insert(Slider {
            track_click: bevy_cgui_widgets::TrackClick::Snap,
        })
        .insert(SliderValue(1.0))
        .insert(SliderRange::new(1.0, 240.0))
        .insert(Node {
            flex_grow: 1.0,
            height: Val::Px(24.0),
            ..default()
        })
        .insert(BackgroundColor(Color::srgba(0.2, 0.2, 0.2, 1.0)))
        .id();
    commands.entity(root).add_child(track);

    // Track overlay: ticks + playhead (absolute positioned)
    let ticks_root = commands
        .spawn_empty()
        .insert(TimelineTicksRoot)
        .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
        .insert(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            ..default()
        })
        .id();
    let playhead = commands
        .spawn_empty()
        .insert(TimelinePlayhead)
        .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
        .insert(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            width: Val::Px(2.0),
            ..default()
        })
        .insert(BackgroundColor(Color::srgba(1.0, 0.25, 0.2, 0.9)))
        .id();
    commands.entity(ticks_root).add_child(playhead);
    commands.entity(track).add_child(ticks_root);

    // Right: Start/End/FPS
    let right_container = commands
        .spawn_empty()
        .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
        .insert(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(6.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .id();
    commands.entity(root).add_child(right_container);

    for (label, field, val, min, max) in [
        ("Start", TimelineField::Start, 1.0, 1.0, 9999.0),
        ("End", TimelineField::End, 240.0, 1.0, 9999.0),
        ("FPS", TimelineField::Fps, 24.0, 1.0, 120.0),
    ] {
        let lbl = commands
            .spawn_empty()
            .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
            .insert(Text::new(label))
            .insert(TextColor(Color::srgba(0.7, 0.7, 0.7, 1.0)))
            .insert(TextFont {
                font_size: FontSize::Px(12.0),
                ..default()
            })
            .id();
        commands.entity(right_container).add_child(lbl);
        let fld = spawn_field_entity(commands, field, val, min, max);
        commands.entity(right_container).add_child(fld);
    }
}

fn spawn_field_entity(
    commands: &mut Commands,
    field: TimelineField,
    value: f32,
    min: f32,
    max: f32,
) -> Entity {
    let fld = commands
        .spawn_empty()
        .insert(field)
        .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
        .insert(NumericInput {
            min,
            max,
            speed: 1.0,
            precision: 0,
        })
        .insert(NumericInputValue(value))
        .insert(Node {
            width: Val::Px(50.0),
            height: Val::Px(24.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border_radius: BorderRadius::all(Val::Px(3.0)),
            ..default()
        })
        .insert(BackgroundColor(Color::srgba(0.18, 0.18, 0.18, 1.0)))
        .insert(Interaction::None)
        .id();
    let txt = commands
        .spawn_empty()
        .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
        .insert(Text::new(format!("{:.0}", value)))
        .insert(TextColor(Color::WHITE))
        .insert(TextFont {
            font_size: FontSize::Px(12.0),
            ..default()
        })
        .id();
    commands.entity(fld).add_child(txt);
    fld
}

/// Sync TimelineState → UI
fn sync_timeline_state_to_ui(
    state: Res<TimelineState>,
    mut commands: Commands,
    track_q: Query<Entity, With<TimelineTrack>>,
    mut field_q: Query<(&TimelineField, &mut NumericInputValue, &Children)>,
    mut text_q: Query<&mut Text>,
    mut icon_q: Query<(Entity, &mut PlayPauseIconContainer, &Children)>,
) {
    if !state.is_changed() {
        return;
    }
    for entity in track_q.iter() {
        commands
            .entity(entity)
            .insert(SliderRange::new(state.start_frame, state.end_frame))
            .insert(SliderValue(state.current_frame));
    }
    for (field, mut v, children) in field_q.iter_mut() {
        let target = match field {
            TimelineField::Current => state.current_frame,
            TimelineField::Start => state.start_frame,
            TimelineField::End => state.end_frame,
            TimelineField::Fps => state.fps,
        };
        v.0 = target;
        for c in children.iter() {
            if let Ok(mut txt) = text_q.get_mut(c) {
                txt.0 = format!("{:.0}", target);
            }
        }
    }
    // Swap Play/Pause icon
    for (icon_e, mut marker, children) in icon_q.iter_mut() {
        let want_play = !state.is_playing;
        if marker.0 == want_play {
            continue;
        }
        marker.0 = want_play;
        for c in children.iter() {
            commands.entity(c).despawn();
        }
        let kind = if want_play {
            IconKind::Play
        } else {
            IconKind::Pause
        };
        let new_icon = spawn_icon(&mut commands, kind, 20.0);
        // Re-take the child nodes and reattach to icon_e
        commands.entity(icon_e).add_child(new_icon);
    }
}

/// Handle button clicks (use Interaction instead of Activate)
fn handle_button_clicks(
    btn_q: Query<(&TimelineButton, &Interaction), Changed<Interaction>>,
    mut state: ResMut<TimelineState>,
) {
    for (btn, interaction) in btn_q.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match btn {
            TimelineButton::First => {
                state.current_frame = state.start_frame;
                state.play_started_at = None;
            }
            TimelineButton::Prev => {
                state.current_frame = (state.current_frame - 1.0).max(state.start_frame);
                state.play_started_at = None;
            }
            TimelineButton::PlayPause => {
                state.is_playing = !state.is_playing;
                state.play_started_at = None;
            }
            TimelineButton::Next => {
                state.current_frame = (state.current_frame + 1.0).min(state.end_frame);
                state.play_started_at = None;
            }
            TimelineButton::Last => {
                state.current_frame = state.end_frame;
                state.play_started_at = None;
            }
        }
    }
}

/// Handle track dragging (Observer)
fn handle_track_value_change(
    event: On<ValueChange<f32>>,
    track_q: Query<Entity, With<TimelineTrack>>,
    mut state: ResMut<TimelineState>,
) {
    if track_q.contains(event.source) {
        state.current_frame = event.value.round();
        state.play_started_at = None;
    }
}

/// Handle numeric field changes (Observer)
fn handle_field_value_change(
    event: On<ValueChange<f32>>,
    field_q: Query<&TimelineField>,
    mut state: ResMut<TimelineState>,
) {
    let Ok(field) = field_q.get(event.source) else {
        return;
    };
    match field {
        TimelineField::Current => {
            state.current_frame = event
                .value
                .round()
                .clamp(state.start_frame, state.end_frame);
            state.play_started_at = None;
        }
        TimelineField::Start => {
            state.start_frame = event.value.round().max(1.0);
        }
        TimelineField::End => {
            state.end_frame = event.value.round().max(state.start_frame + 1.0);
        }
        TimelineField::Fps => {
            state.fps = event.value.clamp(1.0, 120.0);
        }
    }
}

/// Update TimelineUiWantsInput
fn update_timeline_wants_input(
    interaction_q: Query<&Interaction, With<TimelineUiRoot>>,
    focus: Res<InputFocus>,
    field_q: Query<Entity, With<TimelineField>>,
    mut wants: ResMut<TimelineUiWantsInput>,
) {
    let hovered = interaction_q.iter().any(|i| *i != Interaction::None);
    let focused = focus.0.map_or(false, |e| field_q.contains(e));
    wants.0 = hovered || focused;
}

fn nice_step(min_step: u32) -> u32 {
    if min_step <= 1 {
        return 1;
    }
    for &s in &[1, 2, 5, 10, 20, 25, 50, 100, 200, 500, 1000, 2000, 5000] {
        if s >= min_step {
            return s;
        }
    }
    (min_step + 999) / 1000 * 1000
}

fn update_timeline_track_decorations(
    mut commands: Commands,
    state: Res<TimelineState>,
    mut cache: ResMut<TimelineTickCache>,
    track_q: Query<(Entity, &ComputedNode), With<TimelineTrack>>,
    ticks_root_q: Query<(Entity, &Children), With<TimelineTicksRoot>>,
    playhead_q: Query<Entity, With<TimelinePlayhead>>,
) {
    let Ok((track_e, track_node)) = track_q.single() else {
        return;
    };
    let track_w = track_node.size.x.max(1.0) as u32;
    let start = state.start_frame.max(1.0) as u32;
    let end = state.end_frame.max((start + 1) as f32) as u32;
    let range = end.saturating_sub(start).max(1);
    let px_per_frame = track_w as f32 / range as f32;
    let min_px = 70.0_f32;
    let min_step = (min_px / px_per_frame).ceil().max(1.0) as u32;
    let step = nice_step(min_step);
    // Minor ticks: try per-frame when it is visible; otherwise downsample.
    let minor = if px_per_frame >= 6.0 {
        1
    } else if px_per_frame >= 3.0 {
        2
    } else if px_per_frame >= 1.6 {
        5
    } else {
        10
    };

    // Update playhead position (always)
    if let Ok((ticks_root_e, children)) = ticks_root_q.single() {
        for c in children.iter() {
            if playhead_q.contains(c) {
                let f = state.current_frame.round().clamp(start as f32, end as f32) as u32;
                let t = (f - start) as f32 / range as f32;
                commands.entity(c).insert(Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px((t * track_w as f32).round()),
                    top: Val::Px(0.0),
                    bottom: Val::Px(0.0),
                    width: Val::Px(2.0),
                    ..default()
                });
                break;
            }
        }
        // Rebuild ticks only when needed
        if cache.start == start
            && cache.end == end
            && cache.step == step
            && cache.minor == minor
            && cache.track_w == track_w
        {
            return;
        }
        cache.start = start;
        cache.end = end;
        cache.step = step;
        cache.minor = minor;
        cache.track_w = track_w;

        // Clear old children except playhead
        for c in children.iter() {
            if !playhead_q.contains(c) {
                commands.entity(c).despawn();
            }
        }

        // Spawn minor ticks (no labels)
        if minor > 0 {
            let mut ff = start;
            while ff <= end {
                if ff % step != 0 {
                    let t = (ff - start) as f32 / range as f32;
                    let x = (t * track_w as f32).round();
                    let tick = commands
                        .spawn_empty()
                        .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
                        .insert(Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(x),
                            bottom: Val::Px(0.0),
                            width: Val::Px(1.0),
                            height: Val::Px(6.0),
                            ..default()
                        })
                        .insert(BackgroundColor(Color::srgba(0.55, 0.55, 0.55, 0.28)))
                        .id();
                    commands.entity(ticks_root_e).add_child(tick);
                }
                ff = ff.saturating_add(minor);
            }
        }

        // Spawn major ticks + labels
        let mut f = (start / step) * step;
        if f < start {
            f += step;
        }
        while f <= end {
            let t = (f - start) as f32 / range as f32;
            let x = (t * track_w as f32).round();
            let tick = commands
                .spawn_empty()
                .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
                .insert(Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(x),
                    bottom: Val::Px(0.0),
                    width: Val::Px(1.0),
                    height: Val::Px(10.0),
                    ..default()
                })
                .insert(BackgroundColor(Color::srgba(0.6, 0.6, 0.6, 0.55)))
                .id();
            let label = commands
                .spawn_empty()
                .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
                .insert(Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(x + 2.0),
                    top: Val::Px(-14.0),
                    ..default()
                })
                .insert(Text::new(format!("{f}")))
                .insert(TextColor(Color::srgba(0.8, 0.8, 0.8, 0.8)))
                .insert(TextFont {
                    font_size: FontSize::Px(10.0),
                    ..default()
                })
                .id();
            commands.entity(ticks_root_e).add_child(tick);
            commands.entity(ticks_root_e).add_child(label);
            f = f.saturating_add(step);
        }
    } else {
        // If the overlay didn't exist yet (shouldn't happen), respawn UI next frame by invalidating cache.
        cache.start = 0;
        cache.end = 0;
        cache.step = 0;
        cache.minor = 0;
        cache.track_w = 0;
        commands.entity(track_e); // keep borrow checker happy
    }
}
