// placeholder

use bevy::math::{Mat4, Vec3};
use super::super::unity_spline::*;
use super::spline_cache_utility::get_sampled_positions;

/// UI-agnostic helper: build sampled polyline points for drawing spline segments.
/// This mirrors the `SplineCacheUtility.GetCachedPositions` usage in Unity's `SplineHandles.DoSegmentsHandles`.
pub fn build_segment_polyline(spline: &mut Spline, local_to_world: Mat4, samples_per_curve: usize) -> Vec<Vec3> {
    get_sampled_positions(spline, local_to_world, samples_per_curve)
}
