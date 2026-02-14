use bevy::prelude::*;

use crate::libs::geometry::{attrs, ids::PointId};
use crate::mesh::{Attribute, GeoPrimitive, Geometry};
use crate::viewport_options::{DisplayOptions, ViewportViewMode};
use crate::NodeGraphResource;

pub fn draw_polyline_overlay_system(
    node_graph_res: Res<NodeGraphResource>,
    display_options: Res<DisplayOptions>,
    mut giz_front: Gizmos<crate::gizmos::SelectedCurveGizmos>,
    mut giz_xray: Gizmos<crate::gizmos::SelectedCurveXrayGizmos>,
) {
    if matches!(
        display_options.view_mode,
        ViewportViewMode::UV | ViewportViewMode::NodeImage
    ) {
        return;
    }

    let geo = &node_graph_res.0.final_geometry;
    if geo.primitives().is_empty() {
        return;
    }
    let Some(p_attr) = geo.get_point_attribute(attrs::P) else {
        return;
    };

    // Match spline gizmo thickness (SelectedCurveGizmos width), but keep polyline bright white.
    let col_front = Color::srgba(1.0, 1.0, 1.0, 0.98);
    let col_xray = Color::srgba(1.0, 1.0, 1.0, 0.22);

    for prim in geo.primitives().values().iter() {
        let GeoPrimitive::Polyline(line) = prim else {
            continue;
        };
        if line.vertices.len() < 2 {
            continue;
        }

        let mut pts: Vec<Vec3> = Vec::with_capacity(line.vertices.len());
        for &vid in &line.vertices {
            let Some(v) = geo.vertices().get(vid.into()) else {
                continue;
            };
            let Some(p) = sample_point_position(geo, p_attr, v.point_id) else {
                continue;
            };
            pts.push(p);
        }

        if pts.len() < 2 {
            continue;
        }
        for i in 0..(pts.len() - 1) {
            giz_front.line(pts[i], pts[i + 1], col_front);
            giz_xray.line(pts[i], pts[i + 1], col_xray);
        }
        if line.closed && pts.len() > 2 {
            let last = pts[pts.len() - 1];
            giz_front.line(last, pts[0], col_front);
            giz_xray.line(last, pts[0], col_xray);
        }
    }
}

#[inline]
fn sample_point_position(geo: &Geometry, p_attr: &Attribute, pid: PointId) -> Option<Vec3> {
    let dense = geo.points().get_dense_index(pid.into())?;
    sample_vec3_attr(p_attr, dense)
}

#[inline]
fn sample_vec3_attr(attr: &Attribute, index: usize) -> Option<Vec3> {
    if let Some(s) = attr.as_slice::<Vec3>() {
        return s.get(index).copied();
    }
    if let Some(pb) = attr.as_paged::<Vec3>() {
        return pb.get(index);
    }
    None
}
