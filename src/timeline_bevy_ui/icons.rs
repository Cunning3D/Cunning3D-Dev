//! Timeline 播放控制图标 - 用 Bevy UI Node 组合绘制
use super::TIMELINE_UI_LAYER;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

/// 图标类型
#[derive(Clone, Copy)]
pub enum IconKind {
    First,
    Prev,
    Play,
    Pause,
    Next,
    Last,
}

/// 在父节点下生成图标（返回容器 Entity）
pub fn spawn_icon(commands: &mut Commands, kind: IconKind, size: f32) -> Entity {
    let container = commands
        .spawn_empty()
        .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
        .insert(Node {
            width: Val::Px(size),
            height: Val::Px(size),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
        .id();
    let col = Color::WHITE;
    match kind {
        IconKind::Play => {
            spawn_play(commands, container, size, col, false, false);
        }
        IconKind::Pause => {
            spawn_pause_bars(commands, container, size, col);
        }
        IconKind::Prev => {
            spawn_play(commands, container, size, col, true, true);
        }
        IconKind::Next => {
            spawn_play(commands, container, size, col, false, true);
        }
        IconKind::First => {
            spawn_skip(commands, container, size, col, true);
        }
        IconKind::Last => {
            spawn_skip(commands, container, size, col, false);
        }
    }
    container
}

/// 纯矩形拼一个“▶”（可选双箭头）
fn spawn_play(
    commands: &mut Commands,
    parent: Entity,
    size: f32,
    col: Color,
    to_left: bool,
    double: bool,
) {
    let w = size.max(10.0);
    let col_w = (w * 0.12).max(2.0).round();
    let gap = (w * 0.06).max(1.0).round();
    let cols = if double { 10 } else { 6 };
    let tri_cols = cols / if double { 2 } else { 1 };
    let tri_w = tri_cols as f32 * col_w + (tri_cols as f32 - 1.0) * gap;
    let total_w = if double {
        tri_w * 2.0 + gap * 2.0
    } else {
        tri_w
    };
    let left0 = (w - total_w) * 0.5;
    let top0 = w * 0.15;
    let h_max = w * 0.7;

    let spawn_one = |commands: &mut Commands, parent: Entity, x0: f32, to_left: bool| {
        for i in 0..tri_cols {
            let t = (i as f32 / (tri_cols - 1).max(1) as f32).clamp(0.0, 1.0);
            let h = (h_max * (0.35 + 0.65 * t)).round();
            let x = if to_left {
                x0 + (tri_cols as f32 - 1.0 - i as f32) * (col_w + gap)
            } else {
                x0 + i as f32 * (col_w + gap)
            };
            let y = top0 + (h_max - h) * 0.5;
            let bar = commands
                .spawn_empty()
                .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
                .insert(Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(x),
                    top: Val::Px(y),
                    width: Val::Px(col_w),
                    height: Val::Px(h),
                    border_radius: BorderRadius::all(Val::Px(1.0)),
                    ..default()
                })
                .insert(BackgroundColor(col))
                .id();
            commands.entity(parent).add_child(bar);
        }
    };

    spawn_one(commands, parent, left0, to_left);
    if double {
        spawn_one(commands, parent, left0 + tri_w + gap * 2.0, to_left);
    }
}

/// 暂停：两竖条
fn spawn_pause_bars(commands: &mut Commands, parent: Entity, size: f32, col: Color) {
    let bar_w = size * 0.18;
    let bar_h = size * 0.55;
    let gap = size * 0.12;
    for dx in [-gap, gap] {
        let bar = commands
            .spawn_empty()
            .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
            .insert(Node {
                position_type: PositionType::Absolute,
                width: Val::Px(bar_w),
                height: Val::Px(bar_h),
                left: Val::Px(size * 0.5 + dx - bar_w * 0.5),
                top: Val::Px((size - bar_h) * 0.5),
                border_radius: BorderRadius::all(Val::Px(1.0)),
                ..default()
            })
            .insert(BackgroundColor(col))
            .id();
        commands.entity(parent).add_child(bar);
    }
}

/// 跳转到开头/结尾：竖线 + 双箭头
fn spawn_skip(commands: &mut Commands, parent: Entity, size: f32, col: Color, to_start: bool) {
    let w = size.max(10.0);
    let bar_w = (w * 0.12).max(2.0).round();
    let bar_h = (w * 0.7).round();
    let x = if to_start {
        w * 0.14
    } else {
        w - w * 0.14 - bar_w
    };
    let y = (w - bar_h) * 0.5;
    let line = commands
        .spawn_empty()
        .insert(RenderLayers::layer(TIMELINE_UI_LAYER))
        .insert(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(x),
            top: Val::Px(y),
            width: Val::Px(bar_w),
            height: Val::Px(bar_h),
            border_radius: BorderRadius::all(Val::Px(1.0)),
            ..default()
        })
        .insert(BackgroundColor(col))
        .id();
    commands.entity(parent).add_child(line);
    spawn_play(commands, parent, size, col, to_start, true);
}
