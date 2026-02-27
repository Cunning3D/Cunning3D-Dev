//! NumericInput widget - supports keyboard input + drag-to-adjust
use bevy::input::keyboard::{Key, KeyCode, KeyboardInput};
use bevy::input::ButtonState;
use bevy::picking::prelude::*;
use bevy::prelude::*;
use bevy_cgui_widgets::ValueChange;
use bevy_input_focus::{FocusedInput, InputFocus};

/// NumericInput component configuration
#[derive(Component, Clone)]
pub struct NumericInput {
    pub min: f32,
    pub max: f32,
    pub speed: f32,
    pub precision: i32,
}

impl Default for NumericInput {
    fn default() -> Self {
        Self {
            min: f32::MIN,
            max: f32::MAX,
            speed: 1.0,
            precision: 2,
        }
    }
}

/// Current value of NumericInput
#[derive(Component, Clone, Copy, Default)]
pub struct NumericInputValue(pub f32);

/// Edit state
#[derive(Component, Default)]
pub struct NumericInputEditing {
    pub text: String,
    pub original: f32,
}

/// Drag state
#[derive(Component, Default)]
pub struct NumericInputDragState {
    pub dragging: bool,
    pub start_value: f32,
    pub start_x: f32,
}

pub struct NumericInputPlugin;

impl Plugin for NumericInputPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_numeric_click)
            .add_observer(on_numeric_drag_start)
            .add_observer(on_numeric_drag)
            .add_observer(on_numeric_drag_end)
            .add_observer(on_numeric_keyboard);
    }
}

/// Click to enter edit mode
fn on_numeric_click(
    mut click: On<Pointer<Click>>,
    mut commands: Commands,
    q: Query<(&NumericInput, &NumericInputValue), Without<NumericInputEditing>>,
    mut focus: ResMut<InputFocus>,
) {
    let entity = click.event_target();
    let Ok((_, value)) = q.get(entity) else {
        return;
    };
    commands.entity(entity).insert(NumericInputEditing {
        text: format!("{}", value.0),
        original: value.0,
    });
    focus.set(entity);
    click.propagate(false);
}

/// Drag start (when not editing)
fn on_numeric_drag_start(
    mut drag: On<Pointer<DragStart>>,
    mut commands: Commands,
    q: Query<&NumericInputValue, (With<NumericInput>, Without<NumericInputEditing>)>,
) {
    let entity = drag.event_target();
    let Ok(value) = q.get(entity) else { return };
    let pos = drag.pointer_location.position;
    commands.entity(entity).insert(NumericInputDragState {
        dragging: true,
        start_value: value.0,
        start_x: pos.x,
    });
    drag.propagate(false);
}

/// Dragging
fn on_numeric_drag(
    mut drag: On<Pointer<Drag>>,
    mut commands: Commands,
    mut q: Query<(&NumericInput, &mut NumericInputDragState, &Children)>,
    mut text_q: Query<&mut Text>,
) {
    let entity = drag.event_target();
    let Ok((input, mut state, children)) = q.get_mut(entity) else {
        return;
    };
    if !state.dragging {
        return;
    }
    let delta = drag.distance.x * input.speed * 0.1;
    let new_val = (state.start_value + delta).clamp(input.min, input.max);
    let rounded = round_to_precision(new_val, input.precision);
    for c in children.iter() {
        if let Ok(mut txt) = text_q.get_mut(c) {
            txt.0 = format_value(rounded, input.precision);
        }
    }
    commands.trigger(ValueChange {
        source: entity,
        value: rounded,
    });
    drag.propagate(false);
}

/// Drag end
fn on_numeric_drag_end(mut drag: On<Pointer<DragEnd>>, mut q: Query<&mut NumericInputDragState>) {
    let entity = drag.event_target();
    if let Ok(mut state) = q.get_mut(entity) {
        state.dragging = false;
    }
    drag.propagate(false);
}

/// Keyboard input handling
fn on_numeric_keyboard(
    mut event: On<FocusedInput<KeyboardInput>>,
    mut commands: Commands,
    mut q: Query<(&NumericInput, &mut NumericInputEditing, &Children)>,
    mut text_q: Query<&mut Text>,
    mut focus: ResMut<InputFocus>,
) {
    let entity = event.focused_entity;
    let Ok((input, mut editing, children)) = q.get_mut(entity) else {
        return;
    };
    let kb = &event.input;
    if kb.state != ButtonState::Pressed {
        return;
    }

    match kb.key_code {
        KeyCode::Escape => {
            commands.entity(entity).remove::<NumericInputEditing>();
            for c in children.iter() {
                if let Ok(mut txt) = text_q.get_mut(c) {
                    txt.0 = format_value(editing.original, input.precision);
                }
            }
            focus.clear();
            event.propagate(false);
        }
        KeyCode::Enter | KeyCode::NumpadEnter => {
            let parsed: f32 = editing.text.parse().unwrap_or(editing.original);
            let clamped = parsed.clamp(input.min, input.max);
            let rounded = round_to_precision(clamped, input.precision);
            commands.entity(entity).remove::<NumericInputEditing>();
            commands.entity(entity).insert(NumericInputValue(rounded));
            for c in children.iter() {
                if let Ok(mut txt) = text_q.get_mut(c) {
                    txt.0 = format_value(rounded, input.precision);
                }
            }
            commands.trigger(ValueChange {
                source: entity,
                value: rounded,
            });
            focus.clear();
            event.propagate(false);
        }
        KeyCode::Backspace => {
            editing.text.pop();
            for c in children.iter() {
                if let Ok(mut txt) = text_q.get_mut(c) {
                    txt.0 = if editing.text.is_empty() {
                        "0".into()
                    } else {
                        editing.text.clone()
                    };
                }
            }
            event.propagate(false);
        }
        _ => {
            if let Key::Character(ch) = &kb.logical_key {
                let c = ch.chars().next().unwrap_or(' ');
                if c.is_ascii_digit() || c == '.' || c == '-' {
                    editing.text.push(c);
                    for child in children.iter() {
                        if let Ok(mut txt) = text_q.get_mut(child) {
                            txt.0 = editing.text.clone();
                        }
                    }
                    event.propagate(false);
                }
            }
        }
    }
}

fn round_to_precision(v: f32, precision: i32) -> f32 {
    if precision <= 0 {
        v.round()
    } else {
        let factor = 10f32.powi(precision);
        (v * factor).round() / factor
    }
}

fn format_value(v: f32, precision: i32) -> String {
    if precision <= 0 {
        format!("{:.0}", v)
    } else {
        format!("{:.prec$}", v, prec = precision as usize)
    }
}
