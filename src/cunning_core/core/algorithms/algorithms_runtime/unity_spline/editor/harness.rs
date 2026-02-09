use bevy::math::{Mat4, Quat, Vec3};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use super::super::unity_spline::*;
use super::{SplineSelectionState, SelectableElement, TransformContext, PivotMode, HandleOrientation, apply_translation_ctx, apply_rotation_ctx, apply_scale_ctx};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct V3(pub [f32; 3]);

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Q4(pub [f32; 4]);

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct M4(pub [[f32; 4]; 4]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeJson(pub TangentMode);

impl Serialize for ModeJson {
    #[inline] fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.serialize_u8(self.0 as u8) }
}

impl<'de> Deserialize<'de> for ModeJson {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> de::Visitor<'de> for V {
            type Value = ModeJson;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("tangent mode as u8 (0..4) or string") }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> { parse_mode_u64(v).ok_or_else(|| E::custom(format!("invalid tangent mode: {v}"))) }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> { if v < 0 { return Err(E::custom(format!("invalid tangent mode: {v}"))); } self.visit_u64(v as u64) }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> { parse_mode_str(v).ok_or_else(|| E::custom(format!("invalid tangent mode: {v}"))) }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> { self.visit_str(&v) }
        }
        d.deserialize_any(V)
    }
}

#[inline]
fn parse_mode_u64(v: u64) -> Option<ModeJson> {
    Some(ModeJson(match v { 0 => TangentMode::AutoSmooth, 1 => TangentMode::Linear, 2 => TangentMode::Mirrored, 3 => TangentMode::Continuous, 4 => TangentMode::Broken, _ => return None }))
}
#[inline]
fn parse_mode_str(v: &str) -> Option<ModeJson> {
    Some(ModeJson(match v {
        "AutoSmooth" | "Auto" => TangentMode::AutoSmooth,
        "Linear" => TangentMode::Linear,
        "Mirrored" => TangentMode::Mirrored,
        "Continuous" => TangentMode::Continuous,
        "Broken" => TangentMode::Broken,
        _ => return None,
    }))
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct KnotRef(pub [i32; 2]); // [spline, knot]

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SplineContainerSnapshot { pub splines: Vec<SplineSnapshot>, pub links: Vec<Vec<KnotRef>>, pub local_to_world: M4 }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SplineSnapshot { pub closed: bool, pub knots: Vec<KnotSnapshot> }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnotSnapshot { pub position: V3, pub tangent_in: V3, pub tangent_out: V3, pub rotation: Q4, pub mode: ModeJson, pub tension: f32 }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SplineSession {
    pub initial_container: SplineContainer,
    pub ops: Vec<SplineOp>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SplineOp {
    // Selection
    ClearSelection,
    SelectKnot { spline: usize, knot: usize, additive: bool },
    SelectTangent { spline: usize, knot: usize, tangent: BezierTangent, additive: bool },
    SetActiveKnot { spline: usize, knot: usize },

    // Context (tool)
    SetToolContext { pivot: PivotMode, orientation: HandleOrientation, pivot_world: Vec3, handle_rot_world: Quat, move_snap: Vec3, snapping: super::spline_transform_context::SnappingFlags },

    // Transform (selection-driven)
    MoveSelected { delta_world: Vec3 },
    RotateSelected { axis_world: Vec3, delta_rad: f32 },
    ScaleSelected { scale: Vec3 },

    // Direct edits
    SetLocalToWorld { local_to_world: Mat4 },
    SetKnotPositionWorld { spline: usize, knot: usize, world: Vec3 },
    SetKnotRotation { spline: usize, knot: usize, rotation: Quat },
    SetTangentDirectionWorld { spline: usize, knot: usize, tangent: BezierTangent, dir_world: Vec3 },
    SetTangentMode { spline: usize, knot: usize, mode: TangentMode },
    ClearTangent { spline: usize, knot: usize, tangent: BezierTangent },
    SetAutoSmoothTension { spline: usize, knot: usize, tension: f32 },

    // Topology / structure
    InsertOnCurve { spline: usize, curve: usize, t: f32 },
    RemoveKnot { spline: usize, knot: usize },
    ToggleClosed { spline: usize },
    AddKnotToEnd { spline: usize, world: Vec3, normal: Vec3, tangent_out_world: Vec3, default_mode: TangentMode },
    AddKnotToStart { spline: usize, world: Vec3, normal: Vec3, tangent_in_world: Vec3, default_mode: TangentMode },
    CreateKnotOnSurface { spline: usize, dir: DrawingDirection, world: Vec3, normal: Vec3, tangent_out_world: Vec3 },
    CreateKnotOnKnot { spline: usize, dir: DrawingDirection, clicked_spline: usize, clicked_knot: usize, tangent_out_world: Vec3 },

    LinkKnots { a_spline: usize, a_knot: usize, b_spline: usize, b_knot: usize },
    UnlinkKnots { a_spline: usize, a_knot: usize, b_spline: usize, b_knot: usize },
    ReverseFlow { spline: usize },
    DuplicateKnot { spline: usize, knot: usize, target_index: usize },
    DuplicateSpline { spline: usize, from_knot: usize, to_knot: usize },
    SplitSplineOnKnot { spline: usize, knot: usize },
    JoinSplinesOnKnots { a_spline: usize, a_knot: usize, b_spline: usize, b_knot: usize },

    // Assertions (useful for parity tests / debugging)
    AssertKnotCount { spline: usize, count: usize },
    AssertClosed { spline: usize, closed: bool },
    AssertAreLinked { a_spline: usize, a_knot: usize, b_spline: usize, b_knot: usize, linked: bool },
}

pub fn run_session(session: &SplineSession) -> Result<SplineContainer, String> {
    let mut c = session.initial_container.clone();
    let mut sel = SplineSelectionState::default();
    let mut ctx = TransformContext::default();
    run_ops(&mut c, &mut sel, &mut ctx, &session.ops)?;
    Ok(c)
}

pub fn run_ops(container: &mut SplineContainer, selection: &mut SplineSelectionState, ctx: &mut TransformContext, ops: &[SplineOp]) -> Result<(), String> {
    for op in ops {
        match op.clone() {
            SplineOp::ClearSelection => selection.clear(),
            SplineOp::SelectKnot { spline, knot, additive } => {
                let e = SelectableElement::Knot(SelectableKnot { spline_index: spline, knot_index: knot });
                if !additive { selection.clear(); }
                selection.add(e);
                selection.set_active(Some(e));
            }
            SplineOp::SelectTangent { spline, knot, tangent, additive } => {
                let e = SelectableElement::Tangent(SelectableTangent { spline_index: spline, knot_index: knot, tangent });
                if !additive { selection.clear(); }
                selection.add(e);
                selection.set_active(Some(e));
            }
            SplineOp::SetActiveKnot { spline, knot } => selection.set_active(Some(SelectableElement::Knot(SelectableKnot { spline_index: spline, knot_index: knot }))),

            SplineOp::SetToolContext { pivot, orientation, pivot_world, handle_rot_world, move_snap, snapping } => {
                ctx.pivot_mode = pivot;
                ctx.handle_orientation = orientation;
                ctx.pivot_position_world = pivot_world;
                ctx.handle_rotation_world = handle_rot_world;
                ctx.move_snap = move_snap;
                ctx.snapping = snapping;
            }

            SplineOp::MoveSelected { delta_world } => apply_translation_ctx(container, &selection.selected_elements, delta_world, *ctx, None),
            SplineOp::RotateSelected { axis_world, delta_rad } => {
                let axis = if axis_world.length_squared() == 0.0 { Vec3::Y } else { axis_world.normalize() };
                apply_rotation_ctx(container, &selection.selected_elements, Quat::from_axis_angle(axis, delta_rad), *ctx);
            }
            SplineOp::ScaleSelected { scale } => apply_scale_ctx(container, &selection.selected_elements, scale, *ctx),

            SplineOp::SetLocalToWorld { local_to_world } => container.local_to_world = local_to_world,
            SplineOp::SetKnotPositionWorld { spline, knot, world } => {
                if spline >= container.splines.len() || knot >= container.splines[spline].count() { continue; }
                container.splines[spline].knots[knot].position = container.local_to_world.inverse().transform_point3(world);
                container.set_linked_knot_position(SplineKnotIndex::new(spline as i32, knot as i32));
            }
            SplineOp::SetKnotRotation { spline, knot, rotation } => {
                if spline >= container.splines.len() || knot >= container.splines[spline].count() { continue; }
                container.splines[spline].knots[knot].rotation = rotation;
                container.splines[spline].meta[knot].invalidate();
            }
            SplineOp::SetTangentDirectionWorld { spline, knot, tangent, dir_world } => SelectableTangent::set_direction_world(container, spline, knot, tangent, dir_world),
            SplineOp::SetTangentMode { spline, knot, mode } => if spline < container.splines.len() { container.splines[spline].set_tangent_mode_no_notify(knot, mode, BezierTangent::Out); },
            SplineOp::ClearTangent { spline, knot, tangent } => if spline < container.splines.len() { container.splines[spline].clear_tangent(knot, tangent); },
            SplineOp::SetAutoSmoothTension { spline, knot, tension } => if spline < container.splines.len() { container.splines[spline].set_auto_smooth_tension_no_notify(knot, tension); },

            SplineOp::InsertOnCurve { spline, curve, t } => {
                if spline >= container.splines.len() { continue; }
                let count = container.splines[spline].count();
                if count < 2 { continue; }
                let next = if container.splines[spline].closed { (curve + 1) % count } else { (curve + 1).min(count - 1) };
                container.splines[spline].insert_on_curve(next, t);
            }
            SplineOp::RemoveKnot { spline, knot } => if spline < container.splines.len() { container.splines[spline].remove_at(knot); },
            SplineOp::ToggleClosed { spline } => if spline < container.splines.len() { container.splines[spline].closed = !container.splines[spline].closed; },
            SplineOp::AddKnotToEnd { spline, world, normal, tangent_out_world, default_mode } => { let _ = container.add_knot_to_end(spline, world, normal, tangent_out_world, default_mode); }
            SplineOp::AddKnotToStart { spline, world, normal, tangent_in_world, default_mode } => { let _ = container.add_knot_to_start(spline, world, normal, tangent_in_world, default_mode); }
            SplineOp::CreateKnotOnSurface { spline, dir, world, normal, tangent_out_world } => container.create_knot_on_surface(spline, dir, world, normal, tangent_out_world),
            SplineOp::CreateKnotOnKnot { spline, dir, clicked_spline, clicked_knot, tangent_out_world } => container.create_knot_on_knot(spline, dir, SelectableKnot { spline_index: clicked_spline, knot_index: clicked_knot }, tangent_out_world),

            SplineOp::LinkKnots { a_spline, a_knot, b_spline, b_knot } => container.link_knots(SplineKnotIndex::new(a_spline as i32, a_knot as i32), SplineKnotIndex::new(b_spline as i32, b_knot as i32)),
            SplineOp::UnlinkKnots { a_spline, a_knot, b_spline, b_knot } => container.unlink_knots(&[SplineKnotIndex::new(a_spline as i32, a_knot as i32), SplineKnotIndex::new(b_spline as i32, b_knot as i32)]),
            SplineOp::ReverseFlow { spline } => { let _ = container.reverse_flow(spline); }
            SplineOp::DuplicateKnot { spline, knot, target_index } => { let _ = container.duplicate_knot(SplineKnotIndex::new(spline as i32, knot as i32), target_index); }
            SplineOp::DuplicateSpline { spline, from_knot, to_knot } => { let _ = container.duplicate_spline(SplineKnotIndex::new(spline as i32, from_knot as i32), SplineKnotIndex::new(spline as i32, to_knot as i32)); }
            SplineOp::SplitSplineOnKnot { spline, knot } => { let _ = container.split_spline_on_knot(SplineKnotIndex::new(spline as i32, knot as i32)); }
            SplineOp::JoinSplinesOnKnots { a_spline, a_knot, b_spline, b_knot } => { let _ = container.join_splines_on_knots(SplineKnotIndex::new(a_spline as i32, a_knot as i32), SplineKnotIndex::new(b_spline as i32, b_knot as i32)); }

            SplineOp::AssertKnotCount { spline, count } => {
                let got = container.splines.get(spline).map(|s| s.count()).unwrap_or(0);
                if got != count { return Err(format!("AssertKnotCount failed: spline={spline} got={got} expected={count}")); }
            }
            SplineOp::AssertClosed { spline, closed } => {
                let got = container.splines.get(spline).map(|s| s.closed).unwrap_or(false);
                if got != closed { return Err(format!("AssertClosed failed: spline={spline} got={got} expected={closed}")); }
            }
            SplineOp::AssertAreLinked { a_spline, a_knot, b_spline, b_knot, linked } => {
                let got = container.are_knot_linked(SplineKnotIndex::new(a_spline as i32, a_knot as i32), SplineKnotIndex::new(b_spline as i32, b_knot as i32));
                if got != linked { return Err(format!("AssertAreLinked failed: got={got} expected={linked}")); }
            }
        }
    }
    Ok(())
}

pub fn export_json<T: Serialize>(v: &T) -> String { serde_json::to_string_pretty(v).unwrap_or_else(|_| "{}".to_string()) }

pub fn snapshot_container(c: &SplineContainer) -> SplineContainerSnapshot {
    let mut links = c.links.all_links();
    for l in links.iter_mut() { l.sort_by_key(|k| (k.spline, k.knot)); }
    links.sort_by_key(|l| l.first().map(|k| (k.spline, k.knot)).unwrap_or((i32::MAX, i32::MAX)));
    let links: Vec<Vec<KnotRef>> = links.into_iter().map(|g| g.into_iter().map(|k| KnotRef([k.spline, k.knot])).collect()).collect();
    let splines = c.splines.iter().map(|s| {
        let count = s.count();
        let mut knots = Vec::with_capacity(count);
        for i in 0..count {
            let k = s.knots[i];
            let m = s.meta.get(i).map(|x| x.mode).unwrap_or(TangentMode::Broken);
            let t = s.meta.get(i).map(|x| x.tension).unwrap_or(CATMULL_ROM_TENSION);
            knots.push(KnotSnapshot { position: V3(k.position.to_array()), tangent_in: V3(k.tangent_in.to_array()), tangent_out: V3(k.tangent_out.to_array()), rotation: Q4(k.rotation.to_array()), mode: ModeJson(m), tension: t });
        }
        SplineSnapshot { closed: s.closed, knots }
    }).collect();
    SplineContainerSnapshot { splines, links, local_to_world: M4(c.local_to_world.to_cols_array_2d()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cunning_core::core::algorithms::algorithms_runtime::unity_spline::editor::SnappingFlags;

    fn mk_container() -> SplineContainer {
        let mut s = Spline::default();
        s.knots = vec![
            BezierKnot { position: Vec3::ZERO, ..Default::default() },
            BezierKnot { position: Vec3::Z, ..Default::default() },
            BezierKnot { position: Vec3::Z * 2.0, ..Default::default() },
        ];
        s.meta = vec![MetaData::new(TangentMode::Broken, CATMULL_ROM_TENSION); 3];
        let mut c = SplineContainer::default();
        c.splines.push(s);
        c
    }

    #[test]
    fn harness_roundtrip_and_full_ops() {
        let mut c = mk_container();
        let mut sel = SplineSelectionState::default();
        let mut ctx = TransformContext::default();
        let ops = vec![
            SplineOp::SetToolContext { pivot: PivotMode::Pivot, orientation: HandleOrientation::Element, pivot_world: Vec3::ZERO, handle_rot_world: Quat::IDENTITY, move_snap: Vec3::ONE, snapping: SnappingFlags::default() },
            SplineOp::SelectKnot { spline: 0, knot: 1, additive: false },
            SplineOp::MoveSelected { delta_world: Vec3::new(1.0, 0.0, 0.0) },
            SplineOp::RotateSelected { axis_world: Vec3::Y, delta_rad: 0.25 },
            SplineOp::ScaleSelected { scale: Vec3::splat(1.1) },
            SplineOp::InsertOnCurve { spline: 0, curve: 0, t: 0.5 },
            SplineOp::ToggleClosed { spline: 0 },
            SplineOp::AssertKnotCount { spline: 0, count: 4 },
            SplineOp::AssertClosed { spline: 0, closed: true },
        ];
        run_ops(&mut c, &mut sel, &mut ctx, &ops).unwrap();
        let session = SplineSession { initial_container: mk_container(), ops: ops.clone() };
        let json = export_json(&session);
        let back: SplineSession = serde_json::from_str(&json).unwrap();
        let out = run_session(&back).unwrap();
        assert_eq!(out.splines[0].count(), 4);
    }

    #[test]
    fn harness_structural_ops_smoke() {
        let mut c = mk_container();
        // add second spline so join/link ops are exercised
        c.add_spline(c.splines[0].clone());
        let session = SplineSession {
            initial_container: c,
            ops: vec![
                SplineOp::LinkKnots { a_spline: 0, a_knot: 0, b_spline: 1, b_knot: 0 },
                SplineOp::AssertAreLinked { a_spline: 0, a_knot: 0, b_spline: 1, b_knot: 0, linked: true },
                SplineOp::UnlinkKnots { a_spline: 0, a_knot: 0, b_spline: 1, b_knot: 0 },
                SplineOp::AssertAreLinked { a_spline: 0, a_knot: 0, b_spline: 1, b_knot: 0, linked: false },
                SplineOp::ReverseFlow { spline: 0 },
                SplineOp::DuplicateKnot { spline: 0, knot: 0, target_index: 1 },
                SplineOp::AssertKnotCount { spline: 0, count: 4 },
                SplineOp::SplitSplineOnKnot { spline: 0, knot: 1 },
                SplineOp::DuplicateSpline { spline: 1, from_knot: 0, to_knot: 2 },
                SplineOp::JoinSplinesOnKnots { a_spline: 0, a_knot: 0, b_spline: 1, b_knot: 2 },
            ],
        };
        let out = run_session(&session).unwrap();
        assert!(!out.splines.is_empty());
    }
}

