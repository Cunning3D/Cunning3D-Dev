use crate::cunning_core::plugin_system::PluginSystem;
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::cunning_core::registries::tab_registry::TabRegistry;
use crate::cunning_core::scripting::loader::load_rhai_plugins_manual;
use crate::cunning_core::scripting::ScriptEngine;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, WindowPosition};
use bevy_egui::{egui, EguiContexts};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Resource, Clone)]
pub struct SplashSenders {
    pub progress_tx: Sender<f32>,
    pub status_tx: Sender<String>,
}

// Splash 窗口尺寸常量，避免 magic number
pub const SPLASH_WIDTH: f32 = 420.0;
pub const SPLASH_HEIGHT: f32 = 260.0;

// Splash 动画时长常量（秒）
pub const SPLASH_FADE_IN_DURATION: f32 = 0.25;
pub const SPLASH_FADE_OUT_DURATION: f32 = 0.25;
// 最少停留时间：确保用户能看到 Splash（也给首帧白屏一点缓冲）
pub const SPLASH_MIN_HOLD_DURATION: f32 = 2.0;

// 资源：管理 Splash 状态
#[derive(Resource)]
pub struct SplashState {
    pub progress: f32,
    pub status: String,
    pub is_finished: bool,
    // 通道接收端，放到 Mutex 里因为 Resource 需要 Sync
    rx: Arc<Mutex<(Receiver<f32>, Receiver<String>)>>,
}

// 局部动画状态：记录进入 Splash 的时间和完成时间
#[derive(Default)]
struct SplashAnimState {
    started_at: f32,
    finished_at: Option<f32>,
}

// 标记 Splash 阶段是否还在继续（用于控制 State 转换）
#[derive(States, Default, Debug, Clone, Eq, PartialEq, Hash)]
pub enum AppState {
    #[default]
    Splash,
    Editor,
}

pub struct LauncherPlugin;

impl Plugin for LauncherPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppState>();

        let (progress_tx, progress_rx) = channel();
        let (status_tx, status_rx) = channel();

        app.insert_resource(SplashSenders {
            progress_tx,
            status_tx,
        });

        app.insert_resource(SplashState {
            progress: 0.0,
            status: "Initializing...".to_string(),
            is_finished: false,
            // 通道接收端，放到 Mutex 里因为 Resource 需要 Sync
            rx: Arc::new(Mutex::new((progress_rx, status_rx))),
        });

        app.add_systems(OnEnter(AppState::Splash), start_background_loading);
        app.add_systems(Update, splash_ui_system.run_if(in_state(AppState::Splash)));
        app.add_systems(OnEnter(AppState::Editor), transition_to_editor);
    }
}

fn start_background_loading(
    senders: Res<SplashSenders>,
    engine: Res<ScriptEngine>,
    tab_registry: Res<TabRegistry>,
    node_registry: Res<NodeRegistry>,
) {
    let progress_tx = senders.progress_tx.clone();
    let status_tx = senders.status_tx.clone();
    let engine = engine.clone();
    let tab_registry = tab_registry.clone();
    let node_registry = node_registry.clone();

    thread::spawn(move || {
        let _ = status_tx.send("Initializing Kernel...".to_string());
        let _ = progress_tx.send(0.05);

        tab_registry.scan_and_load();
        node_registry.scan_and_load();
        let _ = status_tx.send("Loading Cunning Nodes...".to_string());
        let _ = progress_tx.send(0.35);

        let plugin_dir = "plugins";
        if !Path::new(plugin_dir).exists() {
            if let Err(e) = std::fs::create_dir_all(plugin_dir) {
                bevy::prelude::error!("Failed to create plugin directory {}: {}", plugin_dir, e);
            }
        }
        let plugin_system = PluginSystem::default();
        plugin_system.scan_plugins_latest(plugin_dir, &node_registry);
        let _ = status_tx.send("Loading Native Plugins...".to_string());
        let _ = progress_tx.send(0.6);

        load_rhai_plugins_manual(&engine, &node_registry);
        let _ = status_tx.send("Compiling Script Contexts...".to_string());
        let _ = progress_tx.send(0.95);

        let _ = status_tx.send("Ready".to_string());
        let _ = progress_tx.send(1.0);
    });
}

fn splash_ui_system(
    mut contexts: EguiContexts,
    mut state: ResMut<SplashState>,
    mut next_state: ResMut<NextState<AppState>>,
    time: Res<Time>,
    mut anim: Local<SplashAnimState>,
) {
    let ctx = contexts.ctx_mut();
    // 确保一开始就是深色主题，避免白屏闪一下
    ctx.set_visuals(egui::Visuals::dark());

    // 0. 更新动画时间轴
    let now = time.elapsed_secs();
    if anim.started_at == 0.0 {
        anim.started_at = now;
    }
    let elapsed_since_start = now - anim.started_at;

    // 1. 接收数据（先读到本地变量，避免在持有锁的同时可变借用 state）
    let mut latest_progress: Option<f32> = None;
    let mut latest_status: Option<String> = None;

    if let Ok(mut guard) = state.rx.try_lock() {
        // guard: MutexGuard<(Receiver<f32>, Receiver<String>)>
        while let Ok(p) = guard.0.try_recv() {
            latest_progress = Some(p);
        }
        while let Ok(s) = guard.1.try_recv() {
            latest_status = Some(s);
        }
        // guard 在此作用域结束时被 drop，之后再安全地修改 state
    }

    if let Some(p) = latest_progress {
        state.progress = p;
        if state.progress >= 1.0 {
            state.is_finished = true;
        }
    }

    if let Some(s) = latest_status {
        state.status = s;
    }

    // 1. 计算当前透明度：先淡入，再在完成后淡出
    let mut opacity = 1.0_f32;

    // Fade-in
    if elapsed_since_start < SPLASH_FADE_IN_DURATION {
        opacity = (elapsed_since_start / SPLASH_FADE_IN_DURATION).clamp(0.0, 1.0);
    }

    // Fade-out（只在进度完成后且满足最少停留时间后启动）
    if state.is_finished && elapsed_since_start >= SPLASH_MIN_HOLD_DURATION {
        if anim.finished_at.is_none() {
            anim.finished_at = Some(now);
        }
        if let Some(t0) = anim.finished_at {
            let elapsed_done = now - t0;
            if elapsed_done < SPLASH_FADE_OUT_DURATION {
                let t = (elapsed_done / SPLASH_FADE_OUT_DURATION).clamp(0.0, 1.0);
                let fade_out_opacity = 1.0 - t;
                // 如果已经完成，就以淡出为主导
                opacity = opacity.min(fade_out_opacity);
            } else {
                // 淡出结束，切换到 Editor
                next_state.set(AppState::Editor);
            }
        }
    }

    // 2. 绘制 Splash UI (复刻之前的样式)
    // Bevy 的 egui 默认是在 Window 上覆盖，背景透明需要特殊处理
    // 这里我们可以直接画一个全屏的 CentralPanel，反正 Window 本身是 Splash 大小的

    // 自定义样式
    let panel_frame = egui::Frame::NONE
        .fill(egui::Color32::from_rgb(20, 20, 20))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(20));

    egui::CentralPanel::default()
        .frame(panel_frame)
        .show(ctx, |ui| {
            // 应用整体透明度，实现淡入 / 淡出
            ui.set_opacity(opacity);

            ui.vertical_centered(|ui| {
                ui.add_space(30.0);

                ui.label(
                    egui::RichText::new("CUNNING 3D")
                        .size(48.0)
                        .strong()
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new("FAST AI Naive DCC")
                        .size(16.0)
                        .color(egui::Color32::from_rgb(180, 180, 180)),
                );

                ui.add_space(60.0);

                ui.label(
                    egui::RichText::new(&state.status)
                        .size(14.0)
                        .italics()
                        .color(egui::Color32::from_rgb(150, 150, 150)),
                );

                ui.add_space(10.0);

                // 进度条
                let progress_bar_size = egui::vec2(300.0, 4.0);
                let (rect, _response) =
                    ui.allocate_exact_size(progress_bar_size, egui::Sense::hover());

                ui.painter().rect_filled(
                    rect,
                    egui::CornerRadius::same(2),
                    egui::Color32::from_rgb(40, 40, 40),
                );

                let fill_width = progress_bar_size.x * state.progress.clamp(0.0, 1.0);
                let fill_rect = egui::Rect::from_min_size(
                    rect.min,
                    egui::vec2(fill_width, progress_bar_size.y),
                );

                ui.painter().rect_filled(
                    fill_rect,
                    egui::CornerRadius::same(2),
                    egui::Color32::from_rgb(0, 200, 200),
                );
            });
        });

    // 3. 状态跳转逻辑已经在淡出动画中处理，这里不再直接切换
}

fn transition_to_editor(mut windows: Query<&mut Window>) {
    // 变身！
    if let Ok(mut window) = windows.single_mut() {
        window.title = "Cunning3D 2025".to_string();
        window.decorations = true; // 恢复边框
        window.transparent = false; // 关闭透明（虽然 Bevy 默认 Clear Color 可能会盖住）
                                    // 恢复大小
        window.resolution.set(1280.0, 800.0);
        // 最大化 (Bevy 0.13 API)
        window.mode = bevy::window::WindowMode::Windowed;
        window.set_maximized(true);
        // 居中
        window.position = WindowPosition::Centered(MonitorSelection::Primary);
    }
}
