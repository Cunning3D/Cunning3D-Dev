use bevy::math::{Mat4, Quat, Vec3};
use super::super::unity_spline::*;
use super::spline_element::SelectableElement;
use std::collections::HashSet;
use super::spline_transform_context::{PivotMode, HandleOrientation, TransformContext, TransformContextAdapter};
use super::spline_handle_utility::apply_smart_rounding;
use super::spline_handle_utility::do_increment_snap;

#[derive(Clone, Copy, Debug, Default)]
pub struct RotationSyncData { pub initialized: bool, pub knot_rotation_delta: Quat, pub tangent_local_magnitude_delta: f32, pub scale_multiplier: f32 }

impl RotationSyncData {
    #[inline] pub fn clear(&mut self) { self.initialized = false; self.knot_rotation_delta = Quat::IDENTITY; self.tangent_local_magnitude_delta = 0.0; self.scale_multiplier = 1.0; }
    #[inline] pub fn initialize(&mut self, rot: Quat, mag: f32, scale: f32) { self.initialized = true; self.knot_rotation_delta = rot; self.tangent_local_magnitude_delta = mag; self.scale_multiplier = scale; }
}

/// Minimal subset of UnityEditor.Splines.TransformOperation for runtime-free algorithm parity.
/// This is deliberately UI-agnostic: caller supplies selection + deltas + pivot settings.
pub fn apply_translation(container: &mut SplineContainer, selection: &[SelectableElement], delta: Vec3, _pivot_mode: PivotMode) {
    let mut rotated_knot_cache: HashSet<SelectableKnot> = HashSet::new();
    let mut linked_knot_cache: HashSet<SelectableKnot> = HashSet::new();
    let mut rs = RotationSyncData::default();

    for e in selection {
        match *e {
            SelectableElement::Knot(k) => {
                if k.spline_index >= container.splines.len() || k.knot_index >= container.splines[k.spline_index].count() { continue; }
                if !linked_knot_cache.contains(&k) {
                    let mut knot = container.splines[k.spline_index].knots[k.knot_index];
                    knot.position += delta;
                    container.splines[k.spline_index].set_knot(k.knot_index, knot, BezierTangent::Out);

                    let links = container.links.get_knot_links(SplineKnotIndex::new(k.spline_index as i32, k.knot_index as i32));
                    for lk in links { if lk.is_valid() { linked_knot_cache.insert(SelectableKnot { spline_index: lk.spline as usize, knot_index: lk.knot as usize }); } }

                    container.set_linked_knot_position(SplineKnotIndex::new(k.spline_index as i32, k.knot_index as i32));
                    if !rs.initialized { rs.initialize(Quat::IDENTITY, 0.0, 1.0); }
                }
            }
            SelectableElement::Tangent(t) => {
                if t.spline_index >= container.splines.len() || t.knot_index >= container.splines[t.spline_index].count() { continue; }
                let is_knot_selected = selection.iter().any(|e| matches!(e, SelectableElement::Knot(k) if *k == SelectableKnot { spline_index: t.spline_index, knot_index: t.knot_index }));
                if is_knot_selected { continue; }

                let mode = container.splines[t.spline_index].meta[t.knot_index].mode;
                if mode != TangentMode::Broken && opposite_tangent_selected(selection, t) {
                    container.splines[t.spline_index].set_tangent_mode_no_notify(t.knot_index, TangentMode::Broken, BezierTangent::Out);
                }

                let mode = container.splines[t.spline_index].meta[t.knot_index].mode;
                let knot_world = container.local_to_world.transform_point3(container.splines[t.spline_index].knots[t.knot_index].position);
                let krot = container.splines[t.spline_index].knots[t.knot_index].rotation;
                let local_dir = if t.tangent == BezierTangent::In { container.splines[t.spline_index].knots[t.knot_index].tangent_in } else { container.splines[t.spline_index].knots[t.knot_index].tangent_out };
                let tangent_world = knot_world + krot.mul_vec3(local_dir);
                let target_world = tangent_world + delta;

                if mode == TangentMode::Broken {
                    let mut knot = container.splines[t.spline_index].knots[t.knot_index];
                    apply_position_to_tangent(container.local_to_world, &mut knot, mode, t.tangent, knot_world, tangent_world, target_world);
                    container.splines[t.spline_index].set_knot(t.knot_index, knot, t.tangent);
                    container.set_linked_knot_position(SplineKnotIndex::new(t.spline_index as i32, t.knot_index as i32));
                    if !rs.initialized { rs.initialize(Quat::IDENTITY, 0.0, 1.0); }
                    continue;
                }

                if rotated_knot_cache.contains(&SelectableKnot { spline_index: t.spline_index, knot_index: t.knot_index }) { continue; }

                if !rs.initialized {
                    let (rot_delta, mag_delta) = calculate_mirrored_tangent_translation_deltas(
                        container.local_to_world,
                        krot,
                        knot_world,
                        tangent_world,
                        local_dir,
                        t.tangent,
                        target_world,
                    );
                    rs.initialize(rot_delta, mag_delta, 1.0);
                }

                apply_tangent_rotation_sync_transform(container, t, &mut rs, &mut rotated_knot_cache, true);
                container.set_linked_knot_position(SplineKnotIndex::new(t.spline_index as i32, t.knot_index as i32));
            }
        }
    }

    rs.clear();
}

/// Context-driven translation that matches Unity gating for smart rounding and supports non-identity local_to_world.
pub fn apply_translation_ctx(
    container: &mut SplineContainer,
    selection: &[SelectableElement],
    delta_world: Vec3,
    ctx: TransformContext,
    adapter: Option<&dyn TransformContextAdapter>,
) {
    let mut rotated_knot_cache: HashSet<SelectableKnot> = HashSet::new();
    let mut linked_knot_cache: HashSet<SelectableKnot> = HashSet::new();
    let mut rs = RotationSyncData::default();

    for e in selection {
        match *e {
            SelectableElement::Knot(k) => {
                if k.spline_index >= container.splines.len() || k.knot_index >= container.splines[k.spline_index].count() { continue; }
                if linked_knot_cache.contains(&k) { continue; }

                let mut knot = container.splines[k.spline_index].knots[k.knot_index];
                let knot_world = container.local_to_world.transform_point3(knot.position);
                let mut new_world = knot_world + delta_world;
                if ctx.snapping.incremental_snap_active {
                    new_world = do_increment_snap(new_world, knot_world, ctx.handle_rotation_world, ctx.move_snap);
                }
                if let Some(a) = adapter {
                    new_world = apply_smart_rounding(new_world, a.get_handle_size(new_world), ctx.snapping);
                }
                knot.position = container.local_to_world.inverse().transform_point3(new_world);
                container.splines[k.spline_index].set_knot(k.knot_index, knot, BezierTangent::Out);

                let links = container.links.get_knot_links(SplineKnotIndex::new(k.spline_index as i32, k.knot_index as i32));
                for lk in links { if lk.is_valid() { linked_knot_cache.insert(SelectableKnot { spline_index: lk.spline as usize, knot_index: lk.knot as usize }); } }
                container.set_linked_knot_position(SplineKnotIndex::new(k.spline_index as i32, k.knot_index as i32));
                if !rs.initialized { rs.initialize(Quat::IDENTITY, 0.0, 1.0); }
            }
            SelectableElement::Tangent(t) => {
                if t.spline_index >= container.splines.len() || t.knot_index >= container.splines[t.spline_index].count() { continue; }
                let is_knot_selected = selection.iter().any(|e| matches!(e, SelectableElement::Knot(k) if *k == SelectableKnot { spline_index: t.spline_index, knot_index: t.knot_index }));
                if is_knot_selected { continue; }

                let mode0 = container.splines[t.spline_index].meta[t.knot_index].mode;
                if mode0 != TangentMode::Broken && opposite_tangent_selected(selection, t) {
                    container.splines[t.spline_index].set_tangent_mode_no_notify(t.knot_index, TangentMode::Broken, BezierTangent::Out);
                }
                let mode = container.splines[t.spline_index].meta[t.knot_index].mode;
                let mut knot = container.splines[t.spline_index].knots[t.knot_index];

                let knot_world = container.local_to_world.transform_point3(knot.position);
                let local_dir = if t.tangent == BezierTangent::In { knot.tangent_in } else { knot.tangent_out };
                let tangent_world = knot_world + knot.rotation.mul_vec3(local_dir);
                let mut target_world = tangent_world + delta_world;
                if ctx.snapping.incremental_snap_active {
                    target_world = do_increment_snap(target_world, tangent_world, ctx.handle_rotation_world, ctx.move_snap);
                }
                if let Some(a) = adapter {
                    target_world = apply_smart_rounding(target_world, a.get_handle_size(target_world), ctx.snapping);
                }

                if mode == TangentMode::Broken {
                    apply_position_to_tangent(container.local_to_world, &mut knot, mode, t.tangent, knot_world, tangent_world, target_world);
                    container.splines[t.spline_index].set_knot(t.knot_index, knot, t.tangent);
                    container.set_linked_knot_position(SplineKnotIndex::new(t.spline_index as i32, t.knot_index as i32));
                    if !rs.initialized { rs.initialize(Quat::IDENTITY, 0.0, 1.0); }
                    continue;
                }

                if rotated_knot_cache.contains(&SelectableKnot { spline_index: t.spline_index, knot_index: t.knot_index }) { continue; }
                if !rs.initialized {
                    let (rot_delta, mag_delta) = calculate_mirrored_tangent_translation_deltas(
                        container.local_to_world,
                        knot.rotation,
                        knot_world,
                        tangent_world,
                        local_dir,
                        t.tangent,
                        target_world,
                    );
                    rs.initialize(rot_delta, mag_delta, 1.0);
                }
                apply_tangent_rotation_sync_transform(container, t, &mut rs, &mut rotated_knot_cache, true);
                container.set_linked_knot_position(SplineKnotIndex::new(t.spline_index as i32, t.knot_index as i32));
            }
        }
    }
    rs.clear();
}

#[inline]
fn apply_handle_rotation_to_delta(delta: Quat, handle_rot: Quat) -> Quat { handle_rot * delta * handle_rot.inverse() }

/// Context-driven rotation (covers handle orientation + pivot mode; approximates Unity TransformOperation.ApplyRotation).
pub fn apply_rotation_ctx(
    container: &mut SplineContainer,
    selection: &[SelectableElement],
    delta_rotation: Quat,
    ctx: TransformContext,
) {
    let mut rotated_knot_cache: HashSet<SelectableKnot> = HashSet::new();
    let mut rs = RotationSyncData::default();

    let dr = match ctx.handle_orientation { HandleOrientation::Global => apply_handle_rotation_to_delta(delta_rotation, ctx.handle_rotation_world), _ => delta_rotation };

    for e in selection {
        match *e {
            SelectableElement::Knot(k) => {
                if k.spline_index >= container.splines.len() || k.knot_index >= container.splines[k.spline_index].count() { continue; }
                if rotated_knot_cache.contains(&k) { continue; }
                let mut knot = container.splines[k.spline_index].knots[k.knot_index];
                let prev_rot = knot.rotation;
                if ctx.pivot_mode == PivotMode::Center {
                    let p = container.local_to_world.transform_point3(knot.position);
                    let p2 = ctx.pivot_position_world + dr.mul_vec3(p - ctx.pivot_position_world);
                    knot.position = container.local_to_world.inverse().transform_point3(p2);
                }
                knot.rotation = dr * knot.rotation;
                container.splines[k.spline_index].set_knot(k.knot_index, knot, BezierTangent::Out);
                rotated_knot_cache.insert(k);
                if !rs.initialized { rs.initialize(prev_rot.inverse() * knot.rotation, 0.0, 1.0); }
                container.set_linked_knot_position(SplineKnotIndex::new(k.spline_index as i32, k.knot_index as i32));
            }
            SelectableElement::Tangent(t) => {
                if t.spline_index >= container.splines.len() || t.knot_index >= container.splines[t.spline_index].count() { continue; }
                let is_knot_selected = selection.iter().any(|e| matches!(e, SelectableElement::Knot(k) if *k == SelectableKnot { spline_index: t.spline_index, knot_index: t.knot_index }));
                if is_knot_selected { continue; }
                let mode = container.splines[t.spline_index].meta[t.knot_index].mode;
                if mode == TangentMode::Broken {
                    let knot_world = container.local_to_world.transform_point3(container.splines[t.spline_index].knots[t.knot_index].position);
                    let krot = container.splines[t.spline_index].knots[t.knot_index].rotation;
                    let local_dir = if t.tangent == BezierTangent::In { container.splines[t.spline_index].knots[t.knot_index].tangent_in } else { container.splines[t.spline_index].knots[t.knot_index].tangent_out };
                    let tangent_world = knot_world + krot.mul_vec3(local_dir);
                    let center = if ctx.pivot_mode == PivotMode::Pivot { knot_world } else { ctx.pivot_position_world };
                    let target_world = center + dr.mul_vec3(tangent_world - center);
                    let mut knot = container.splines[t.spline_index].knots[t.knot_index];
                    apply_position_to_tangent(container.local_to_world, &mut knot, mode, t.tangent, knot_world, tangent_world, target_world);
                    container.splines[t.spline_index].set_knot(t.knot_index, knot, t.tangent);
                    container.set_linked_knot_position(SplineKnotIndex::new(t.spline_index as i32, t.knot_index as i32));
                    continue;
                }

                if rotated_knot_cache.contains(&SelectableKnot { spline_index: t.spline_index, knot_index: t.knot_index }) { continue; }
                if !rs.initialized { rs.initialize(dr, 0.0, 1.0); }
                apply_tangent_rotation_sync_transform(container, t, &mut rs, &mut rotated_knot_cache, false);
                container.set_linked_knot_position(SplineKnotIndex::new(t.spline_index as i32, t.knot_index as i32));
            }
        }
    }
    rs.clear();
}

/// Context-driven scale (covers handle orientation + pivot mode; approximates Unity TransformOperation.ApplyScale).
pub fn apply_scale_ctx(
    container: &mut SplineContainer,
    selection: &[SelectableElement],
    scale: Vec3,
    ctx: TransformContext,
) {
    let mut rotated_knot_cache: HashSet<SelectableKnot> = HashSet::new();
    let mut rs = RotationSyncData::default();
    let inv = ctx.handle_rotation_world.inverse();

    for e in selection {
        match *e {
            SelectableElement::Knot(k) => {
                if k.spline_index >= container.splines.len() || k.knot_index >= container.splines[k.spline_index].count() { continue; }
                if rotated_knot_cache.contains(&k) { continue; }
                let mut knot = container.splines[k.spline_index].knots[k.knot_index];
                if ctx.pivot_mode == PivotMode::Center {
                    let p = container.local_to_world.transform_point3(knot.position);
                    let delta = inv.mul_vec3(p - ctx.pivot_position_world);
                    let scaled = Vec3::new(delta.x * scale.x, delta.y * scale.y, delta.z * scale.z);
                    let p2 = ctx.pivot_position_world + ctx.handle_rotation_world.mul_vec3(scaled);
                    knot.position = container.local_to_world.inverse().transform_point3(p2);
                }
                let in_dir = inv.mul_vec3(knot.tangent_in);
                let out_dir = inv.mul_vec3(knot.tangent_out);
                knot.tangent_in = ctx.handle_rotation_world.mul_vec3(Vec3::new(in_dir.x * scale.x, in_dir.y * scale.y, in_dir.z * scale.z));
                knot.tangent_out = ctx.handle_rotation_world.mul_vec3(Vec3::new(out_dir.x * scale.x, out_dir.y * scale.y, out_dir.z * scale.z));
                container.splines[k.spline_index].set_knot(k.knot_index, knot, BezierTangent::Out);
                rotated_knot_cache.insert(k);
                if !rs.initialized { rs.initialize(Quat::IDENTITY, 0.0, 1.0); }
                container.set_linked_knot_position(SplineKnotIndex::new(k.spline_index as i32, k.knot_index as i32));
            }
            SelectableElement::Tangent(t) => {
                if t.spline_index >= container.splines.len() || t.knot_index >= container.splines[t.spline_index].count() { continue; }
                let is_knot_selected = selection.iter().any(|e| matches!(e, SelectableElement::Knot(k) if *k == SelectableKnot { spline_index: t.spline_index, knot_index: t.knot_index }));
                if is_knot_selected { continue; }

                let mode = container.splines[t.spline_index].meta[t.knot_index].mode;
                let knot_world = container.local_to_world.transform_point3(container.splines[t.spline_index].knots[t.knot_index].position);
                let krot = container.splines[t.spline_index].knots[t.knot_index].rotation;
                let local_dir = if t.tangent == BezierTangent::In { container.splines[t.spline_index].knots[t.knot_index].tangent_in } else { container.splines[t.spline_index].knots[t.knot_index].tangent_out };
                let tangent_world = knot_world + krot.mul_vec3(local_dir);
                let center = if ctx.pivot_mode == PivotMode::Center { ctx.pivot_position_world } else { knot_world };
                let dp = inv.mul_vec3(tangent_world - center);
                let scaled = Vec3::new(dp.x * scale.x, dp.y * scale.y, dp.z * scale.z);
                let target_world = center + ctx.handle_rotation_world.mul_vec3(scaled);

                if mode == TangentMode::Broken {
                    let mut knot = container.splines[t.spline_index].knots[t.knot_index];
                    apply_position_to_tangent(container.local_to_world, &mut knot, mode, t.tangent, knot_world, tangent_world, target_world);
                    container.splines[t.spline_index].set_knot(t.knot_index, knot, t.tangent);
                    container.set_linked_knot_position(SplineKnotIndex::new(t.spline_index as i32, t.knot_index as i32));
                    continue;
                }

                if rotated_knot_cache.contains(&SelectableKnot { spline_index: t.spline_index, knot_index: t.knot_index }) { continue; }
                if !rs.initialized { rs.initialize(Quat::IDENTITY, 0.0, 1.0); }
                apply_tangent_rotation_sync_transform(container, t, &mut rs, &mut rotated_knot_cache, false);
                container.set_linked_knot_position(SplineKnotIndex::new(t.spline_index as i32, t.knot_index as i32));
            }
        }
    }
    rs.clear();
}

/// UI-agnostic port of `TransformOperation.ApplyRotation` for Element-handle orientation.
pub fn apply_rotation(container: &mut SplineContainer, selection: &[SelectableElement], delta_rotation: Quat, rotation_center: Vec3, pivot_mode: PivotMode) {
    let mut rotated_knot_cache: HashSet<SelectableKnot> = HashSet::new();
    let mut rs = RotationSyncData::default();

    for e in selection {
        match *e {
            SelectableElement::Knot(k) => {
                if k.spline_index >= container.splines.len() || k.knot_index >= container.splines[k.spline_index].count() { continue; }
                if rotated_knot_cache.contains(&k) { continue; }
                let mut knot = container.splines[k.spline_index].knots[k.knot_index];
                let prev_rot = knot.rotation;
                if pivot_mode == PivotMode::Center {
                    let p = container.local_to_world.transform_point3(knot.position);
                    let p2 = rotation_center + delta_rotation.mul_vec3(p - rotation_center);
                    knot.position = container.local_to_world.inverse().transform_point3(p2);
                }
                knot.rotation = delta_rotation * knot.rotation;
                container.splines[k.spline_index].set_knot(k.knot_index, knot, BezierTangent::Out);
                rotated_knot_cache.insert(k);
                if !rs.initialized { rs.initialize(prev_rot.inverse() * knot.rotation, 0.0, 1.0); }
                container.set_linked_knot_position(SplineKnotIndex::new(k.spline_index as i32, k.knot_index as i32));
            }
            SelectableElement::Tangent(t) => {
                if t.spline_index >= container.splines.len() || t.knot_index >= container.splines[t.spline_index].count() { continue; }
                let is_knot_selected = selection.iter().any(|e| matches!(e, SelectableElement::Knot(k) if *k == SelectableKnot { spline_index: t.spline_index, knot_index: t.knot_index }));
                if is_knot_selected { continue; }

                let mode = container.splines[t.spline_index].meta[t.knot_index].mode;
                if mode == TangentMode::Broken {
                    let knot_world = container.local_to_world.transform_point3(container.splines[t.spline_index].knots[t.knot_index].position);
                    let krot = container.splines[t.spline_index].knots[t.knot_index].rotation;
                    let local_dir = if t.tangent == BezierTangent::In { container.splines[t.spline_index].knots[t.knot_index].tangent_in } else { container.splines[t.spline_index].knots[t.knot_index].tangent_out };
                    let tangent_world = knot_world + krot.mul_vec3(local_dir);
                    let center = if pivot_mode == PivotMode::Pivot { knot_world } else { rotation_center };
                    let target_world = center + delta_rotation.mul_vec3(tangent_world - center);
                    let mut knot = container.splines[t.spline_index].knots[t.knot_index];
                    apply_position_to_tangent(container.local_to_world, &mut knot, mode, t.tangent, knot_world, tangent_world, target_world);
                    container.splines[t.spline_index].set_knot(t.knot_index, knot, t.tangent);
                    container.set_linked_knot_position(SplineKnotIndex::new(t.spline_index as i32, t.knot_index as i32));
                    continue;
                }

                if rotated_knot_cache.contains(&SelectableKnot { spline_index: t.spline_index, knot_index: t.knot_index }) { continue; }
                if !rs.initialized { rs.initialize(delta_rotation, 0.0, 1.0); }
                apply_tangent_rotation_sync_transform(container, t, &mut rs, &mut rotated_knot_cache, false);
                container.set_linked_knot_position(SplineKnotIndex::new(t.spline_index as i32, t.knot_index as i32));
            }
        }
    }

    rs.clear();
}

/// UI-agnostic port of `TransformOperation.ApplyScale` (Element orientation only; caller provides pivot position and handle rotation).
pub fn apply_scale(container: &mut SplineContainer, selection: &[SelectableElement], scale: Vec3, pivot: Vec3, handle_rotation: Quat, pivot_mode: PivotMode) {
    let mut rotated_knot_cache: HashSet<SelectableKnot> = HashSet::new();
    let mut rs = RotationSyncData::default();
    let inv = handle_rotation.inverse();

    for e in selection {
        match *e {
            SelectableElement::Knot(k) => {
                if k.spline_index >= container.splines.len() || k.knot_index >= container.splines[k.spline_index].count() { continue; }
                if rotated_knot_cache.contains(&k) { continue; }
                let mut knot = container.splines[k.spline_index].knots[k.knot_index];
                if pivot_mode == PivotMode::Center {
                    let p = container.local_to_world.transform_point3(knot.position);
                    let delta = inv.mul_vec3(p - pivot);
                    let scaled = Vec3::new(delta.x * scale.x, delta.y * scale.y, delta.z * scale.z);
                    let p2 = pivot + handle_rotation.mul_vec3(scaled);
                    knot.position = container.local_to_world.inverse().transform_point3(p2);
                }
                // scale tangents from current values around handle axes
                let in_dir = inv.mul_vec3(knot.tangent_in);
                let out_dir = inv.mul_vec3(knot.tangent_out);
                knot.tangent_in = handle_rotation.mul_vec3(Vec3::new(in_dir.x * scale.x, in_dir.y * scale.y, in_dir.z * scale.z));
                knot.tangent_out = handle_rotation.mul_vec3(Vec3::new(out_dir.x * scale.x, out_dir.y * scale.y, out_dir.z * scale.z));
                container.splines[k.spline_index].set_knot(k.knot_index, knot, BezierTangent::Out);
                rotated_knot_cache.insert(k);
                if !rs.initialized { rs.initialize(Quat::IDENTITY, 0.0, 1.0); }
                container.set_linked_knot_position(SplineKnotIndex::new(k.spline_index as i32, k.knot_index as i32));
            }
            SelectableElement::Tangent(t) => {
                if t.spline_index >= container.splines.len() || t.knot_index >= container.splines[t.spline_index].count() { continue; }
                let is_knot_selected = selection.iter().any(|e| matches!(e, SelectableElement::Knot(k) if *k == SelectableKnot { spline_index: t.spline_index, knot_index: t.knot_index }));
                if is_knot_selected { continue; }

                let mode = container.splines[t.spline_index].meta[t.knot_index].mode;
                let knot_world = container.local_to_world.transform_point3(container.splines[t.spline_index].knots[t.knot_index].position);
                let krot = container.splines[t.spline_index].knots[t.knot_index].rotation;
                let local_dir = if t.tangent == BezierTangent::In { container.splines[t.spline_index].knots[t.knot_index].tangent_in } else { container.splines[t.spline_index].knots[t.knot_index].tangent_out };
                let tangent_world = knot_world + krot.mul_vec3(local_dir);

                let scale_center = if pivot_mode == PivotMode::Center { pivot } else { knot_world };
                let dp = inv.mul_vec3(tangent_world - scale_center);
                let scaled = Vec3::new(dp.x * scale.x, dp.y * scale.y, dp.z * scale.z);
                let target_world = scale_center + handle_rotation.mul_vec3(scaled);

                if mode == TangentMode::Broken {
                    let mut knot = container.splines[t.spline_index].knots[t.knot_index];
                    apply_position_to_tangent(container.local_to_world, &mut knot, mode, t.tangent, knot_world, tangent_world, target_world);
                    container.splines[t.spline_index].set_knot(t.knot_index, knot, t.tangent);
                    container.set_linked_knot_position(SplineKnotIndex::new(t.spline_index as i32, t.knot_index as i32));
                    continue;
                }

                if rotated_knot_cache.contains(&SelectableKnot { spline_index: t.spline_index, knot_index: t.knot_index }) { continue; }
                if !rs.initialized { rs.initialize(Quat::IDENTITY, 0.0, 1.0); }
                apply_tangent_rotation_sync_transform(container, t, &mut rs, &mut rotated_knot_cache, false);
                container.set_linked_knot_position(SplineKnotIndex::new(t.spline_index as i32, t.knot_index as i32));
            }
        }
    }

    rs.clear();
}

#[inline]
fn opposite_tangent_selected(selection: &[SelectableElement], tangent: SelectableTangent) -> bool {
    let opp = SelectableTangent { spline_index: tangent.spline_index, knot_index: tangent.knot_index, tangent: if tangent.tangent == BezierTangent::In { BezierTangent::Out } else { BezierTangent::In } };
    selection.iter().any(|e| matches!(e, SelectableElement::Tangent(t) if *t == opp))
}

fn apply_tangent_rotation_sync_transform(container: &mut SplineContainer, tangent: SelectableTangent, rs: &mut RotationSyncData, rotated_knot_cache: &mut HashSet<SelectableKnot>, absolute_scale: bool) {
    let si = tangent.spline_index;
    let ki = tangent.knot_index;
    if si >= container.splines.len() || ki >= container.splines[si].count() { return; }

    let mode = container.splines[si].meta[ki].mode;
    let is_active = true; // Caller decides "currentElementSelected"; for now assume the manipulated tangent is active.
    if is_active || mode == TangentMode::Mirrored || (!absolute_scale && mode == TangentMode::Continuous) {
        let mut k = container.splines[si].knots[ki];
        let mut ld = if tangent.tangent == BezierTangent::In { k.tangent_in } else { k.tangent_out };
        if absolute_scale {
            if ld.length() == 0.0 { ld = Vec3::new(0.0, 0.0, 1.0); }
            ld += ld.normalize_or_zero() * rs.tangent_local_magnitude_delta;
        } else {
            ld *= rs.scale_multiplier;
        }
        if tangent.tangent == BezierTangent::In { k.tangent_in = ld; } else { k.tangent_out = ld; }
        k.rotation = rs.knot_rotation_delta * k.rotation;
        container.splines[si].set_knot(ki, k, tangent.tangent);
    } else {
        let mut k = container.splines[si].knots[ki];
        k.rotation = rs.knot_rotation_delta * k.rotation;
        container.splines[si].set_knot(ki, k, tangent.tangent);
    }

    rotated_knot_cache.insert(SelectableKnot { spline_index: si, knot_index: ki });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_linked_knots_moved_once() {
        let mut c = SplineContainer::default();
        let mut s = Spline::default();
        s.knots = vec![BezierKnot { position: Vec3::ZERO, ..Default::default() }, BezierKnot { position: Vec3::X, ..Default::default() }];
        s.meta = vec![MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION); 2];
        c.splines.push(s);
        c.link_knots(SplineKnotIndex::new(0, 0), SplineKnotIndex::new(0, 1));

        let sel = [SelectableElement::Knot(SelectableKnot { spline_index: 0, knot_index: 0 }), SelectableElement::Knot(SelectableKnot { spline_index: 0, knot_index: 1 })];
        apply_translation(&mut c, &sel, Vec3::Z, PivotMode::Pivot);
        assert_eq!(c.splines[0].knots[0].position, Vec3::Z);
        assert_eq!(c.splines[0].knots[1].position, Vec3::Z);
    }

    #[derive(Clone, Copy, Debug)]
    struct DummyAdapter;
    impl TransformContextAdapter for DummyAdapter {
        fn get_handle_size(&self, _world_pos: Vec3) -> f32 { 10.0 }
    }

    #[test]
    fn translation_ctx_respects_local_to_world() {
        let mut c = SplineContainer::default();
        c.local_to_world = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
        let mut s = Spline::default();
        s.knots = vec![BezierKnot { position: Vec3::ZERO, ..Default::default() }, BezierKnot { position: Vec3::Z, ..Default::default() }];
        s.meta = vec![MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION); 2];
        c.splines.push(s);
        let sel = [SelectableElement::Knot(SelectableKnot { spline_index: 0, knot_index: 0 })];
        let ctx = TransformContext::default();
        apply_translation_ctx(&mut c, &sel, Vec3::X, ctx, Some(&DummyAdapter));
        assert!((c.splines[0].knots[0].position.x - 1.0).abs() < 1e-6);
    }

    #[test]
    fn rotation_ctx_center_moves_position() {
        let mut c = SplineContainer::default();
        let mut s = Spline::default();
        s.knots = vec![BezierKnot { position: Vec3::X, ..Default::default() }, BezierKnot { position: Vec3::Z, ..Default::default() }];
        s.meta = vec![MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION); 2];
        c.splines.push(s);
        let sel = [SelectableElement::Knot(SelectableKnot { spline_index: 0, knot_index: 0 })];
        let mut ctx = TransformContext::default();
        ctx.pivot_mode = PivotMode::Center;
        ctx.pivot_position_world = Vec3::ZERO;
        let dr = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        apply_rotation_ctx(&mut c, &sel, dr, ctx);
        let p = c.splines[0].knots[0].position;
        assert!(p.distance(Vec3::new(0.0, 0.0, -1.0)) < 1e-3);
    }
}
