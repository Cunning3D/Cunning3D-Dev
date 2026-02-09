use bevy::camera::visibility::{InheritedVisibility, ViewVisibility, VisibilityClass};
use bevy::prelude::*;
use bevy::render::sync_world::RenderEntity;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::{
    render::{normal::NormalMarker, point::PointMarker},
    scene::components::{PrimitiveNormalTag, VertexNormalTag},
    viewport_options::DisplayOptions,
};

static LAST_NORMAL_STATE: AtomicU32 = AtomicU32::new(u32::MAX);
static LAST_NORMAL_VIS: AtomicU32 = AtomicU32::new(u32::MAX);

pub(crate) fn update_point_visibility_system(
    display_options: Res<DisplayOptions>,
    mut query_points: Query<&mut Visibility, With<PointMarker>>,
) {
    let new_visibility = if display_options.overlays.show_points { Visibility::Visible } else { Visibility::Hidden };
    for mut visibility in &mut query_points {
        *visibility = new_visibility;
    }
}

pub(crate) fn debug_normal_entity_state(
    q: Query<
        (
            &Visibility,
            Option<&InheritedVisibility>,
            Option<&ViewVisibility>,
            Option<&VisibilityClass>,
            Option<&RenderEntity>,
            Option<&GlobalTransform>,
            Option<&Mesh3d>,
        ),
        With<NormalMarker>,
    >,
) {
    let (mut total, mut has_render, mut has_view_vis, mut view_vis_true, mut has_global, mut has_mesh) =
        (0u32, 0u32, 0u32, 0u32, 0u32, 0u32);
    for (_vis, _inh, view_vis, _class, render, global, mesh) in &q {
        total += 1;
        if render.is_some() {
            has_render += 1;
        }
        if view_vis.is_some() {
            has_view_vis += 1;
            if view_vis.unwrap().get() {
                view_vis_true += 1;
            }
        }
        if global.is_some() {
            has_global += 1;
        }
        if mesh.is_some() {
            has_mesh += 1;
        }
    }
    let packed = total
        | (has_render << 8)
        | (has_view_vis << 16)
        | (view_vis_true << 20)
        | (has_global << 24)
        | (has_mesh << 28);
    let prev = LAST_NORMAL_STATE.swap(packed, Ordering::Relaxed);
    if prev != packed {
        info!(
            "[Normal] state total={} has_render={} has_view_vis={} view_vis_true={} has_global={} has_mesh={}",
            total, has_render, has_view_vis, view_vis_true, has_global, has_mesh
        );
    }
}

pub(crate) fn update_normal_visibility_system(
    display_options: Res<DisplayOptions>,
    mut query: Query<(&mut Visibility, Option<&VertexNormalTag>, Option<&PrimitiveNormalTag>), With<NormalMarker>>,
) {
    let vertex_vis = if display_options.overlays.show_vertex_normals { Visibility::Visible } else { Visibility::Hidden };
    let prim_vis = if display_options.overlays.show_primitive_normals { Visibility::Visible } else { Visibility::Hidden };
    let mut touched = 0u32;
    for (mut visibility, vertex_tag, prim_tag) in &mut query {
        if vertex_tag.is_some() {
            *visibility = vertex_vis;
            touched += 1;
        } else if prim_tag.is_some() {
            *visibility = prim_vis;
            touched += 1;
        }
    }
    let packed =
        (display_options.overlays.show_vertex_normals as u32) | ((display_options.overlays.show_primitive_normals as u32) << 1) | (touched << 2);
    let prev = LAST_NORMAL_VIS.swap(packed, Ordering::Relaxed);
    if prev != packed {
        info!(
            "[Normal] update_normal_visibility vertex={} prim={} touched={}",
            display_options.overlays.show_vertex_normals,
            display_options.overlays.show_primitive_normals,
            touched
        );
    }
}

