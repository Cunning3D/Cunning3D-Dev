use bevy::math::{Mat4, Vec3};
use super::super::unity_spline::*;

/// UI-agnostic equivalent of UnityEditor.Splines.Utilities.SplineCacheUtility.GetCachedPositions.
/// We intentionally implement the sampling algorithm; caching policy can be layered on top later.
pub fn get_sampled_positions(spline: &mut Spline, local_to_world: Mat4, samples_per_curve: usize) -> Vec<Vec3> {
    let count = spline.count();
    if count < 2 { return Vec::new(); }
    let curves = if spline.closed { count } else { count - 1 };
    let spc = samples_per_curve.max(2);
    let mut out = Vec::with_capacity(curves * spc);
    for ci in 0..curves {
        let curve = spline.get_curve(ci);
        for s in 0..spc {
            let t = s as f32 / (spc as f32 - 1.0);
            out.push(local_to_world.transform_point3(evaluate_position(curve, t)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampled_positions_count_open() {
        let mut s = Spline::default();
        s.knots = vec![BezierKnot { position: Vec3::ZERO, ..Default::default() }, BezierKnot { position: Vec3::Z, ..Default::default() }, BezierKnot { position: Vec3::Z * 2.0, ..Default::default() }];
        s.meta = vec![MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION); 3];
        s.closed = false;
        let pts = get_sampled_positions(&mut s, Mat4::IDENTITY, 8);
        assert_eq!(pts.len(), (3 - 1) * 8);
    }

    #[test]
    fn sampled_positions_count_closed() {
        let mut s = Spline::default();
        s.knots = vec![BezierKnot { position: Vec3::ZERO, ..Default::default() }, BezierKnot { position: Vec3::Z, ..Default::default() }, BezierKnot { position: Vec3::Z * 2.0, ..Default::default() }];
        s.meta = vec![MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION); 3];
        s.closed = true;
        let pts = get_sampled_positions(&mut s, Mat4::IDENTITY, 8);
        assert_eq!(pts.len(), 3 * 8);
    }
}
