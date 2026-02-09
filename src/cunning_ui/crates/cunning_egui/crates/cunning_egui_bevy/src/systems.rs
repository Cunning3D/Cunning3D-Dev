use crate::{
    EguiContext, EguiContextQuery, EguiContextQueryItem, EguiInput, EguiSettings, WindowSize,
};
use bevy::{
    ecs::{
        message::{MessageReader, MessageWriter},
        query::QueryEntityError,
        system::{Local, Res, SystemParam},
    },
    input::{
        keyboard::{Key, KeyCode, KeyboardInput},
        mouse::{MouseButton, MouseButtonInput, MouseScrollUnit, MouseWheel},
        touch::TouchInput,
        ButtonState,
    },
    log,
    prelude::{Commands, Entity, Query, Resource, Time},
    time::Real,
    window::{CursorMoved, CursorIcon, RequestRedraw, SystemCursorIcon},
};
use std::collections::HashMap;
use bevy::window::Ime;

#[allow(missing_docs)]
#[derive(SystemParam)]
// IMPORTANT: remember to add the logic to clear event readers to the `clear` method.
pub struct InputEvents<'w, 's> {
    pub ev_cursor: MessageReader<'w, 's, CursorMoved>,
    pub ev_mouse_button_input: MessageReader<'w, 's, MouseButtonInput>,
    pub ev_mouse_wheel: MessageReader<'w, 's, MouseWheel>,
    pub ev_keyboard_input: MessageReader<'w, 's, KeyboardInput>,
    pub ev_ime: MessageReader<'w, 's, Ime>,
    pub ev_touch: MessageReader<'w, 's, TouchInput>,
}

impl<'w, 's> InputEvents<'w, 's> {
    /// Consumes all the events.
    pub fn clear(&mut self) {
        self.ev_cursor.read().last();
        self.ev_mouse_button_input.read().last();
        self.ev_mouse_wheel.read().last();
        self.ev_keyboard_input.read().last();
        self.ev_ime.read().last();
        self.ev_touch.read().last();
    }
}

/// Stores "pressed" state of modifier keys.
/// Will be removed if Bevy adds support for `ButtonInput<Key>` (logical keys).
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct ModifierKeysState {
    shift: bool,
    ctrl: bool,
    alt: bool,
    win: bool,
}

#[allow(missing_docs)]
#[derive(SystemParam)]
pub struct InputResources<'w, 's> {
    #[cfg(all(
        feature = "manage_clipboard",
        not(target_os = "android"),
        not(all(target_arch = "wasm32", not(web_sys_unstable_apis)))
    ))]
    pub egui_clipboard: bevy::ecs::system::ResMut<'w, crate::EguiClipboard>,
    pub modifier_keys_state: Local<'s, ModifierKeysState>,
    pub time: Res<'w, Time<Real>>,
}

#[allow(missing_docs)]
#[derive(SystemParam)]
pub struct ContextSystemParams<'w, 's> {
    pub contexts: Query<'w, 's, EguiContextQuery>,
    pub is_macos: Local<'s, bool>,
}

impl<'w, 's> ContextSystemParams<'w, 's> {
    fn with_window_context<R>(
        &mut self,
        window: Entity,
        f: impl FnOnce(EguiContextQueryItem<'_, '_>) -> R,
    ) -> Option<R> {
        match self.contexts.get_mut(window) {
            Ok(context) => Some(f(context)),
            Err(err @ QueryEntityError::AliasedMutability(_)) => {
                panic!("Failed to get an Egui context for a window ({window:?}): {err:?}");
            }
            Err(err) => {
                log::error!("Failed to get an Egui context for a window ({window:?}): {err:?}");
                None
            }
        }
    }
}

/// Processes Bevy input and feeds it to Egui.
pub fn process_input_system(
    mut input_events: InputEvents,
    mut input_resources: InputResources,
    mut context_params: ContextSystemParams,
    egui_settings: Res<EguiSettings>,
    occlusion: Res<crate::EguiOcclusionRects>,
    mut redraw: MessageWriter<RequestRedraw>,
    mut ime_sent_enabled: Local<HashMap<Entity, bool>>,
) {
    // Test whether it's macOS or OS X.
    use std::sync::Once;
    static START: Once = Once::new();
    START.call_once(|| {
        // The default for WASM is `false` since the `target_os` is `unknown`.
        *context_params.is_macos = cfg!(target_os = "macos");

        #[cfg(target_arch = "wasm32")]
        if let Some(window) = web_sys::window() {
            let nav = window.navigator();
            if let Ok(user_agent) = nav.user_agent() {
                if user_agent.to_ascii_lowercase().contains("mac") {
                    *context_params.is_macos = true;
                }
            }
        }
    });

    let mut keyboard_input_events = Vec::new();
    for event in input_events.ev_keyboard_input.read() {
        // Copy the events as we might want to pass them to an Egui context later.
        keyboard_input_events.push(event.clone());

        let KeyboardInput {
            logical_key, state, ..
        } = event;
        match logical_key {
            Key::Shift => {
                input_resources.modifier_keys_state.shift = state.is_pressed();
            }
            Key::Control => {
                input_resources.modifier_keys_state.ctrl = state.is_pressed();
            }
            Key::Alt => {
                input_resources.modifier_keys_state.alt = state.is_pressed();
            }
            Key::Super | Key::Meta => {
                input_resources.modifier_keys_state.win = state.is_pressed();
            }
            _ => {}
        };
    }

    let ModifierKeysState {
        shift,
        ctrl,
        alt,
        win,
    } = *input_resources.modifier_keys_state;
    let mac_cmd = if *context_params.is_macos { win } else { false };
    let command = if *context_params.is_macos { win } else { ctrl };

    let modifiers = egui::Modifiers {
        alt,
        ctrl,
        shift,
        mac_cmd,
        command,
    };

    for event in input_events.ev_cursor.read() {
        let scale_factor = egui_settings.scale_factor;
        let (x, y): (f32, f32) = (event.position / scale_factor).into();
        let mouse_position = egui::pos2(x, y);
        let _ = context_params.with_window_context(event.window, |mut window_context| {
            window_context.ctx.mouse_position = mouse_position;
            let blocked = occlusion
                .0
                .get(&event.window.index().index())
                .map_or(false, |rs| rs.iter().any(|r| r.contains(mouse_position)));
            window_context.egui_input.events.push(if blocked { egui::Event::PointerGone } else { egui::Event::PointerMoved(mouse_position) });
            // Reactive mode: hover/highlight needs a redraw on pointer movement (when not occluded by a hole).
            if !blocked { redraw.write(RequestRedraw); }
        });
    }

    for event in input_events.ev_mouse_button_input.read() {
        let button = match event.button {
            MouseButton::Left => Some(egui::PointerButton::Primary),
            MouseButton::Right => Some(egui::PointerButton::Secondary),
            MouseButton::Middle => Some(egui::PointerButton::Middle),
            _ => None,
        };
        let pressed = match event.state {
            ButtonState::Pressed => true,
            ButtonState::Released => false,
        };
        let _ = context_params.with_window_context(event.window, |mut window_context| {
            let blocked = occlusion
                .0
                .get(&event.window.index().index())
                .map_or(false, |rs| rs.iter().any(|r| r.contains(window_context.ctx.mouse_position)));
            if blocked { return; }
            if let Some(button) = button {
                window_context
                    .egui_input
                    .events
                    .push(egui::Event::PointerButton {
                        pos: window_context.ctx.mouse_position,
                        button,
                        pressed,
                        modifiers,
                    });
                // Reactive mode: clicks must trigger a redraw or menus will feel inconsistent.
                redraw.write(RequestRedraw);
            }
        });
    }

    for event in input_events.ev_mouse_wheel.read() {
        let delta = egui::vec2(event.x, event.y);
        let unit = match event.unit {
            MouseScrollUnit::Line => egui::MouseWheelUnit::Line,
            MouseScrollUnit::Pixel => egui::MouseWheelUnit::Point,
        };
        let _ = context_params.with_window_context(event.window, |mut window_context| {
            let blocked = occlusion
                .0
                .get(&event.window.index().index())
                .map_or(false, |rs| rs.iter().any(|r| r.contains(window_context.ctx.mouse_position)));
            if blocked { return; }
            window_context.egui_input.events.push(egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
            });
            // Reactive mode: wheel scrolling must trigger an explicit redraw to avoid flicker.
            redraw.write(RequestRedraw);
        });
    }

    if !command && !win || !*context_params.is_macos && ctrl && alt {
        for event in keyboard_input_events.iter() {
            if !event.state.is_pressed() {
                continue;
            }
            // Avoid duplicate text insertion while IME is active: commits come via `Ime::Commit`.
            if *ime_sent_enabled.get(&event.window).unwrap_or(&false) {
                continue;
            }
            let Some(text) = event.text.as_deref() else { continue; };
            if text.chars().all(char::is_control) {
                continue;
            }
            let _ = context_params.with_window_context(event.window, |mut window_context| {
                window_context
                    .egui_input
                    .events
                    .push(egui::Event::Text(text.to_string()));
            });
        }
    }

    for event in keyboard_input_events {
        let (Some(key), physical_key) = (
            bevy_to_egui_key(&event.logical_key),
            bevy_to_egui_physical_key(&event.key_code),
        ) else {
            continue;
        };

        let _ = context_params.with_window_context(event.window, |mut window_context| {
            let egui_event = egui::Event::Key {
                key,
                pressed: event.state.is_pressed(),
                repeat: false,
                modifiers,
                physical_key,
            };
            window_context.egui_input.events.push(egui_event);
        });

        // We also check that it's an `ButtonState::Pressed` event, as we don't want to
        // copy, cut or paste on the key release.
        #[cfg(all(
            feature = "manage_clipboard",
            not(target_os = "android"),
            not(target_arch = "wasm32")
        ))]
        if command && event.state.is_pressed() {
            let _ = context_params.with_window_context(event.window, |mut window_context| {
                match key {
                    egui::Key::C => window_context.egui_input.events.push(egui::Event::Copy),
                    egui::Key::X => window_context.egui_input.events.push(egui::Event::Cut),
                    egui::Key::V => {
                        if let Some(contents) = input_resources.egui_clipboard.get_contents() {
                            log::info!(
                                "[bevy_egui] Ctrl+V clipboard contents: {:?} (bytes: {:?})",
                                contents,
                                contents.as_bytes()
                            );
                            window_context
                                .egui_input
                                .events
                                .push(egui::Event::Text(contents))
                        }
                    }
                    _ => {}
                }
            });
        }
    }

    // IME input (composition + commit), required for CJK input on Windows/macOS/Linux.
    for event in input_events.ev_ime.read() {
        macro_rules! emit_ime_enabled {
            ($window:expr, $enabled:expr) => {{
                let window: Entity = $window;
                let enabled: bool = $enabled;
                let already = *ime_sent_enabled.get(&window).unwrap_or(&false);
                if enabled && !already {
                    let _ = context_params.with_window_context(window, |mut window_context| {
                        window_context.egui_input.events.push(egui::Event::Ime(egui::ImeEvent::Enabled));
                    });
                    ime_sent_enabled.insert(window, true);
                } else if !enabled && already {
                    let _ = context_params.with_window_context(window, |mut window_context| {
                        window_context.egui_input.events.push(egui::Event::Ime(egui::ImeEvent::Disabled));
                    });
                    ime_sent_enabled.insert(window, false);
                }
            }};
        }

        match event {
            Ime::Enabled { window } => emit_ime_enabled!(*window, true),
            Ime::Disabled { window } => emit_ime_enabled!(*window, false),
            Ime::Preedit { window, value, cursor } => {
                if cursor.is_some() {
                    emit_ime_enabled!(*window, true);
                    let _ = context_params.with_window_context(*window, |mut window_context| {
                        window_context.egui_input.events.push(egui::Event::Ime(egui::ImeEvent::Preedit(value.clone())));
                    });
                } else {
                    emit_ime_enabled!(*window, false);
                }
            }
            Ime::Commit { window, value } => {
                let _ = context_params.with_window_context(*window, |mut window_context| {
                    window_context.egui_input.events.push(egui::Event::Ime(egui::ImeEvent::Commit(value.clone())));
                });
                emit_ime_enabled!(*window, false);
            }
        }

        // Reactive mode: IME updates must repaint to display preedit text/caret.
        redraw.write(RequestRedraw);
    }

    #[cfg(all(
        feature = "manage_clipboard",
        target_arch = "wasm32",
        web_sys_unstable_apis
    ))]
    while let Some(event) = input_resources.egui_clipboard.try_receive_clipboard_event() {
        // In web, we assume that we have only 1 window per app.
        let mut window_context = context_params.contexts.single_mut();

        match event {
            crate::web_clipboard::WebClipboardEvent::Copy => {
                window_context.egui_input.events.push(egui::Event::Copy);
            }
            crate::web_clipboard::WebClipboardEvent::Cut => {
                window_context.egui_input.events.push(egui::Event::Cut);
            }
            crate::web_clipboard::WebClipboardEvent::Paste(contents) => {
                input_resources
                    .egui_clipboard
                    .set_contents_internal(&contents);
                window_context
                    .egui_input
                    .events
                    .push(egui::Event::Text(contents))
            }
        }
    }

    for event in input_events.ev_touch.read() {
        let touch_id = egui::TouchId::from(event.id);
        let scale_factor = egui_settings.scale_factor;
        let touch_position: (f32, f32) = (event.position / scale_factor).into();

        let _ = context_params.with_window_context(event.window, |mut window_context| {
            // Emit touch event
            window_context.egui_input.events.push(egui::Event::Touch {
                device_id: egui::TouchDeviceId(event.window.to_bits()),
                id: touch_id,
                phase: match event.phase {
                    bevy::input::touch::TouchPhase::Started => egui::TouchPhase::Start,
                    bevy::input::touch::TouchPhase::Moved => egui::TouchPhase::Move,
                    bevy::input::touch::TouchPhase::Ended => egui::TouchPhase::End,
                    bevy::input::touch::TouchPhase::Canceled => egui::TouchPhase::Cancel,
                },
                pos: egui::pos2(touch_position.0, touch_position.1),
                force: match event.force {
                    Some(bevy::input::touch::ForceTouch::Normalized(force)) => Some(force as f32),
                    Some(bevy::input::touch::ForceTouch::Calibrated {
                        force,
                        max_possible_force,
                        ..
                    }) => Some((force / max_possible_force) as f32),
                    None => None,
                },
            });

            // Emulate mouse.
            if window_context.ctx.pointer_touch_id.is_none()
                || window_context.ctx.pointer_touch_id.unwrap() == event.id
            {
                match event.phase {
                    bevy::input::touch::TouchPhase::Started => {
                        window_context.ctx.pointer_touch_id = Some(event.id);
                        window_context.egui_input.events.push(egui::Event::PointerMoved(
                            egui::pos2(touch_position.0, touch_position.1),
                        ));
                        window_context
                            .egui_input
                            .events
                            .push(egui::Event::PointerButton {
                                pos: egui::pos2(touch_position.0, touch_position.1),
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                modifiers,
                            });
                    }
                    bevy::input::touch::TouchPhase::Moved => {
                        window_context.egui_input.events.push(egui::Event::PointerMoved(
                            egui::pos2(touch_position.0, touch_position.1),
                        ));
                    }
                    bevy::input::touch::TouchPhase::Ended => {
                        window_context.ctx.pointer_touch_id = None;
                        window_context
                            .egui_input
                            .events
                            .push(egui::Event::PointerButton {
                                pos: egui::pos2(touch_position.0, touch_position.1),
                                button: egui::PointerButton::Primary,
                                pressed: false,
                                modifiers,
                            });
                        window_context.egui_input.events.push(egui::Event::PointerGone);
                    }
                    bevy::input::touch::TouchPhase::Canceled => {
                        window_context.ctx.pointer_touch_id = None;
                        window_context.egui_input.events.push(egui::Event::PointerGone);
                    }
                }
            }
        });
    }

    for mut context in context_params.contexts.iter_mut() {
        context.egui_input.modifiers = modifiers;
        context.egui_input.time = Some(input_resources.time.elapsed_secs_f64());
    }

    // In some cases, we may skip certain events. For example, we ignore `ReceivedCharacter` events
    // when alt or ctrl button is pressed. We still want to clear event buffer.
    input_events.clear();
}

/// Initialises Egui contexts (for multiple windows).
pub fn update_window_contexts_system(
    mut context_params: ContextSystemParams,
    egui_settings: Res<EguiSettings>,
) {
    for mut context in context_params.contexts.iter_mut() {
        let new_window_size = WindowSize::new(
            context.window.physical_width() as f32,
            context.window.physical_height() as f32,
            context.window.scale_factor(),
        );
        let width = new_window_size.physical_width
            / new_window_size.scale_factor
            / egui_settings.scale_factor;
        let height = new_window_size.physical_height
            / new_window_size.scale_factor
            / egui_settings.scale_factor;

        if width < 1.0 || height < 1.0 {
            continue;
        }

        context.egui_input.screen_rect = Some(egui::Rect::from_min_max(
            egui::pos2(0.0, 0.0),
            egui::pos2(width, height),
        ));

        context
            .ctx
            .get_mut()
            .set_pixels_per_point(new_window_size.scale_factor * egui_settings.scale_factor);

        *context.window_size = new_window_size;
    }
}

/// Marks frame start for Egui.
pub fn begin_frame_system(mut contexts: Query<(&mut EguiContext, &mut EguiInput)>) {
    for (mut ctx, mut egui_input) in contexts.iter_mut() {
        ctx.get_mut().begin_frame(egui_input.take());
    }
}

/// Reads Egui output.
pub fn process_output_system(
    mut commands: Commands,
    #[cfg_attr(not(feature = "open_url"), allow(unused_variables))] egui_settings: Res<
        EguiSettings,
    >,
    mut contexts: Query<EguiContextQuery>,
    #[cfg(all(feature = "manage_clipboard", not(target_os = "android")))]
    mut egui_clipboard: bevy::ecs::system::ResMut<crate::EguiClipboard>,
    mut event: MessageWriter<RequestRedraw>,
    #[cfg(windows)] mut last_cursor_icon: Local<HashMap<Entity, egui::CursorIcon>>,
    mut ime_state: Local<HashMap<Entity, (bool, Option<egui::Rect>)>>,
) {
    for mut context in contexts.iter_mut() {
        let ctx = context.ctx.get_mut();
        let full_output = ctx.end_frame();
        let egui::FullOutput {
            platform_output,
            shapes,
            textures_delta,
            pixels_per_point,
            viewport_output,
        } = full_output;
        let paint_jobs = ctx.tessellate(shapes, pixels_per_point);

        context.render_output.paint_jobs = paint_jobs;
        context.render_output.textures_delta.append(textures_delta);

        context.egui_output.platform_output = platform_output.clone();

        // IME support (allow + cursor area)
        {
            let ime_allowed = platform_output.ime.is_some();
            let entry = ime_state.entry(context.window_entity).or_insert((false, None));
            if entry.0 != ime_allowed || entry.1 != platform_output.ime.map(|i| i.rect) {
                bevy_winit::WINIT_WINDOWS.with_borrow_mut(|winit_windows| {
                    let Some(winit_window) = winit_windows.get_window(context.window_entity) else { return; };
                    if entry.0 != ime_allowed {
                        winit_window.set_ime_allowed(ime_allowed);
                        entry.0 = ime_allowed;
                    }
                    if let Some(ime) = platform_output.ime {
                        let rect = ime.rect;
                        let ime_rect_px = rect * pixels_per_point;
                        if entry.1 != Some(rect) {
                            entry.1 = Some(rect);
                            #[cfg(not(target_arch = "wasm32"))]
                            winit_window.set_ime_cursor_area(
                                winit::dpi::Position::Physical(winit::dpi::PhysicalPosition::new(
                                    ime_rect_px.min.x.round() as i32,
                                    ime_rect_px.min.y.round() as i32,
                                )),
                                winit::dpi::Size::Physical(winit::dpi::PhysicalSize::new(
                                    ime_rect_px.width().round() as u32,
                                    ime_rect_px.height().round() as u32,
                                )),
                            );
                        }
                    } else {
                        entry.1 = None;
                    }
                });
            }
        }

        #[cfg(all(
            feature = "manage_clipboard",
            not(target_os = "android"),
            not(all(target_arch = "wasm32", not(web_sys_unstable_apis)))
        ))]
        for command in &platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                if !text.is_empty() {
                    egui_clipboard.set_contents(text);
                }
            }
        }

        let mut set_icon = || {
            let sys = egui_to_winit_cursor_icon(platform_output.cursor_icon)
                .unwrap_or(SystemCursorIcon::Default);
            commands.entity(context.window_entity).insert(CursorIcon::from(sys));
        };

        #[cfg(windows)]
        {
            let last_cursor_icon = last_cursor_icon.entry(context.window_entity).or_default();
            if *last_cursor_icon != platform_output.cursor_icon {
                set_icon();
                *last_cursor_icon = platform_output.cursor_icon;
            }
        }
        #[cfg(not(windows))]
        set_icon();

        if ctx.has_requested_repaint() {
            let delay = viewport_output
                .get(&ctx.viewport_id())
                .map(|o| o.repaint_delay)
                .unwrap_or(std::time::Duration::ZERO);
            if delay == std::time::Duration::ZERO {
                event.write(RequestRedraw);
            } else {
                // `WinitWakeUp` no longer exists in Bevy 0.18-dev. Request a redraw immediately.
                event.write(RequestRedraw);
            }
        }

        #[cfg(feature = "open_url")]
        if let Some(egui::output::OpenUrl { url, new_tab }) = platform_output.open_url {
            let target = if new_tab {
                "_blank"
            } else {
                egui_settings
                    .default_open_url_target
                    .as_deref()
                    .unwrap_or("_self")
            };
            if let Err(err) = webbrowser::open_browser_with_options(
                webbrowser::Browser::Default,
                &url,
                webbrowser::BrowserOptions::new().with_target_hint(target),
            ) {
                log::error!("Failed to open '{}': {:?}", url, err);
            }
        }
    }
}

fn egui_to_winit_cursor_icon(cursor_icon: egui::CursorIcon) -> Option<SystemCursorIcon> {
    match cursor_icon {
        egui::CursorIcon::Default => Some(SystemCursorIcon::Default),
        egui::CursorIcon::PointingHand => Some(SystemCursorIcon::Pointer),
        egui::CursorIcon::ResizeHorizontal => Some(SystemCursorIcon::EwResize),
        egui::CursorIcon::ResizeNeSw => Some(SystemCursorIcon::NeswResize),
        egui::CursorIcon::ResizeNwSe => Some(SystemCursorIcon::NwseResize),
        egui::CursorIcon::ResizeVertical => Some(SystemCursorIcon::NsResize),
        egui::CursorIcon::Text => Some(SystemCursorIcon::Text),
        egui::CursorIcon::Grab => Some(SystemCursorIcon::Grab),
        egui::CursorIcon::Grabbing => Some(SystemCursorIcon::Grabbing),
        egui::CursorIcon::ContextMenu => Some(SystemCursorIcon::ContextMenu),
        egui::CursorIcon::Help => Some(SystemCursorIcon::Help),
        egui::CursorIcon::Progress => Some(SystemCursorIcon::Progress),
        egui::CursorIcon::Wait => Some(SystemCursorIcon::Wait),
        egui::CursorIcon::Cell => Some(SystemCursorIcon::Cell),
        egui::CursorIcon::Crosshair => Some(SystemCursorIcon::Crosshair),
        egui::CursorIcon::VerticalText => Some(SystemCursorIcon::VerticalText),
        egui::CursorIcon::Alias => Some(SystemCursorIcon::Alias),
        egui::CursorIcon::Copy => Some(SystemCursorIcon::Copy),
        egui::CursorIcon::Move => Some(SystemCursorIcon::Move),
        egui::CursorIcon::NoDrop => Some(SystemCursorIcon::NoDrop),
        egui::CursorIcon::NotAllowed => Some(SystemCursorIcon::NotAllowed),
        egui::CursorIcon::AllScroll => Some(SystemCursorIcon::AllScroll),
        egui::CursorIcon::ZoomIn => Some(SystemCursorIcon::ZoomIn),
        egui::CursorIcon::ZoomOut => Some(SystemCursorIcon::ZoomOut),
        egui::CursorIcon::ResizeEast => Some(SystemCursorIcon::EResize),
        egui::CursorIcon::ResizeSouthEast => Some(SystemCursorIcon::SeResize),
        egui::CursorIcon::ResizeSouth => Some(SystemCursorIcon::SResize),
        egui::CursorIcon::ResizeSouthWest => Some(SystemCursorIcon::SwResize),
        egui::CursorIcon::ResizeWest => Some(SystemCursorIcon::WResize),
        egui::CursorIcon::ResizeNorthWest => Some(SystemCursorIcon::NwResize),
        egui::CursorIcon::ResizeNorth => Some(SystemCursorIcon::NResize),
        egui::CursorIcon::ResizeNorthEast => Some(SystemCursorIcon::NeResize),
        egui::CursorIcon::ResizeColumn => Some(SystemCursorIcon::ColResize),
        egui::CursorIcon::ResizeRow => Some(SystemCursorIcon::RowResize),
        egui::CursorIcon::None => None,
    }
}

/// Matches the implementation of <https://github.com/emilk/egui/blob/68b3ef7f6badfe893d3bbb1f791b481069d807d9/crates/egui-winit/src/lib.rs#L1005>.
pub fn bevy_to_egui_key(key: &Key) -> Option<egui::Key> {
    let key = match key {
        Key::Character(str) => return egui::Key::from_name(str.as_str()),
        Key::Unidentified(_) | Key::Dead(_) => return None,

        Key::Enter => egui::Key::Enter,
        Key::Tab => egui::Key::Tab,
        Key::Space => egui::Key::Space,
        Key::ArrowDown => egui::Key::ArrowDown,
        Key::ArrowLeft => egui::Key::ArrowLeft,
        Key::ArrowRight => egui::Key::ArrowRight,
        Key::ArrowUp => egui::Key::ArrowUp,
        Key::End => egui::Key::End,
        Key::Home => egui::Key::Home,
        Key::PageDown => egui::Key::PageDown,
        Key::PageUp => egui::Key::PageUp,
        Key::Backspace => egui::Key::Backspace,
        Key::Delete => egui::Key::Delete,
        Key::Insert => egui::Key::Insert,
        Key::Escape => egui::Key::Escape,
        Key::F1 => egui::Key::F1,
        Key::F2 => egui::Key::F2,
        Key::F3 => egui::Key::F3,
        Key::F4 => egui::Key::F4,
        Key::F5 => egui::Key::F5,
        Key::F6 => egui::Key::F6,
        Key::F7 => egui::Key::F7,
        Key::F8 => egui::Key::F8,
        Key::F9 => egui::Key::F9,
        Key::F10 => egui::Key::F10,
        Key::F11 => egui::Key::F11,
        Key::F12 => egui::Key::F12,
        Key::F13 => egui::Key::F13,
        Key::F14 => egui::Key::F14,
        Key::F15 => egui::Key::F15,
        Key::F16 => egui::Key::F16,
        Key::F17 => egui::Key::F17,
        Key::F18 => egui::Key::F18,
        Key::F19 => egui::Key::F19,
        Key::F20 => egui::Key::F20,

        _ => return None,
    };
    Some(key)
}

/// Matches the implementation of <https://github.com/emilk/egui/blob/68b3ef7f6badfe893d3bbb1f791b481069d807d9/crates/egui-winit/src/lib.rs#L1080>.
pub fn bevy_to_egui_physical_key(key: &KeyCode) -> Option<egui::Key> {
    let key = match key {
        KeyCode::ArrowDown => egui::Key::ArrowDown,
        KeyCode::ArrowLeft => egui::Key::ArrowLeft,
        KeyCode::ArrowRight => egui::Key::ArrowRight,
        KeyCode::ArrowUp => egui::Key::ArrowUp,

        KeyCode::Escape => egui::Key::Escape,
        KeyCode::Tab => egui::Key::Tab,
        KeyCode::Backspace => egui::Key::Backspace,
        KeyCode::Enter | KeyCode::NumpadEnter => egui::Key::Enter,

        KeyCode::Insert => egui::Key::Insert,
        KeyCode::Delete => egui::Key::Delete,
        KeyCode::Home => egui::Key::Home,
        KeyCode::End => egui::Key::End,
        KeyCode::PageUp => egui::Key::PageUp,
        KeyCode::PageDown => egui::Key::PageDown,

        // Punctuation
        KeyCode::Space => egui::Key::Space,
        KeyCode::Comma => egui::Key::Comma,
        KeyCode::Period => egui::Key::Period,
        // KeyCode::Colon => egui::Key::Colon, // NOTE: there is no physical colon key on an american keyboard
        KeyCode::Semicolon => egui::Key::Semicolon,
        KeyCode::Backslash => egui::Key::Backslash,
        KeyCode::Slash | KeyCode::NumpadDivide => egui::Key::Slash,
        KeyCode::BracketLeft => egui::Key::OpenBracket,
        KeyCode::BracketRight => egui::Key::CloseBracket,
        KeyCode::Backquote => egui::Key::Backtick,

        KeyCode::Cut => egui::Key::Cut,
        KeyCode::Copy => egui::Key::Copy,
        KeyCode::Paste => egui::Key::Paste,
        KeyCode::Minus | KeyCode::NumpadSubtract => egui::Key::Minus,
        KeyCode::NumpadAdd => egui::Key::Plus,
        KeyCode::Equal => egui::Key::Equals,

        KeyCode::Digit0 | KeyCode::Numpad0 => egui::Key::Num0,
        KeyCode::Digit1 | KeyCode::Numpad1 => egui::Key::Num1,
        KeyCode::Digit2 | KeyCode::Numpad2 => egui::Key::Num2,
        KeyCode::Digit3 | KeyCode::Numpad3 => egui::Key::Num3,
        KeyCode::Digit4 | KeyCode::Numpad4 => egui::Key::Num4,
        KeyCode::Digit5 | KeyCode::Numpad5 => egui::Key::Num5,
        KeyCode::Digit6 | KeyCode::Numpad6 => egui::Key::Num6,
        KeyCode::Digit7 | KeyCode::Numpad7 => egui::Key::Num7,
        KeyCode::Digit8 | KeyCode::Numpad8 => egui::Key::Num8,
        KeyCode::Digit9 | KeyCode::Numpad9 => egui::Key::Num9,

        KeyCode::KeyA => egui::Key::A,
        KeyCode::KeyB => egui::Key::B,
        KeyCode::KeyC => egui::Key::C,
        KeyCode::KeyD => egui::Key::D,
        KeyCode::KeyE => egui::Key::E,
        KeyCode::KeyF => egui::Key::F,
        KeyCode::KeyG => egui::Key::G,
        KeyCode::KeyH => egui::Key::H,
        KeyCode::KeyI => egui::Key::I,
        KeyCode::KeyJ => egui::Key::J,
        KeyCode::KeyK => egui::Key::K,
        KeyCode::KeyL => egui::Key::L,
        KeyCode::KeyM => egui::Key::M,
        KeyCode::KeyN => egui::Key::N,
        KeyCode::KeyO => egui::Key::O,
        KeyCode::KeyP => egui::Key::P,
        KeyCode::KeyQ => egui::Key::Q,
        KeyCode::KeyR => egui::Key::R,
        KeyCode::KeyS => egui::Key::S,
        KeyCode::KeyT => egui::Key::T,
        KeyCode::KeyU => egui::Key::U,
        KeyCode::KeyV => egui::Key::V,
        KeyCode::KeyW => egui::Key::W,
        KeyCode::KeyX => egui::Key::X,
        KeyCode::KeyY => egui::Key::Y,
        KeyCode::KeyZ => egui::Key::Z,

        KeyCode::F1 => egui::Key::F1,
        KeyCode::F2 => egui::Key::F2,
        KeyCode::F3 => egui::Key::F3,
        KeyCode::F4 => egui::Key::F4,
        KeyCode::F5 => egui::Key::F5,
        KeyCode::F6 => egui::Key::F6,
        KeyCode::F7 => egui::Key::F7,
        KeyCode::F8 => egui::Key::F8,
        KeyCode::F9 => egui::Key::F9,
        KeyCode::F10 => egui::Key::F10,
        KeyCode::F11 => egui::Key::F11,
        KeyCode::F12 => egui::Key::F12,
        KeyCode::F13 => egui::Key::F13,
        KeyCode::F14 => egui::Key::F14,
        KeyCode::F15 => egui::Key::F15,
        KeyCode::F16 => egui::Key::F16,
        KeyCode::F17 => egui::Key::F17,
        KeyCode::F18 => egui::Key::F18,
        KeyCode::F19 => egui::Key::F19,
        KeyCode::F20 => egui::Key::F20,
        _ => return None,
    };
    Some(key)
}
