use crate::libs::geometry::group::ElementGroupMask;
use bevy::prelude::Vec3;

const INDEX_MUL: u64 = 0x9E37_79B9_7F4A_7C15;
const AXIS_MUL: u64 = 0xBF58_476D_1CE4_E5B9;
const AX_X: u64 = 0x1234_5678_9ABC_DEF0u64.wrapping_mul(AXIS_MUL);
const AX_Y: u64 = 0x0FED_CBA9_8765_4321u64.wrapping_mul(AXIS_MUL);
const AX_Z: u64 = 0xA5A5_A5A5_5A5A_5A5Au64.wrapping_mul(AXIS_MUL);
const U32_TO_F01: f32 = 1.0 / (u32::MAX as f32);

#[inline]
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[inline]
fn rand_signed(h: u64) -> f32 {
    // Houdini Point Jitter semantics: each component in [-0.5, 0.5].
    (h as u32) as f32 * U32_TO_F01 - 0.5
}

#[inline]
fn jitter_xyz(seed: u64, point_index: usize) -> (f32, f32, f32) {
    let base = seed ^ (point_index as u64).wrapping_mul(INDEX_MUL);
    (
        rand_signed(splitmix64(base ^ AX_X)),
        rand_signed(splitmix64(base ^ AX_Y)),
        rand_signed(splitmix64(base ^ AX_Z)),
    )
}

#[inline]
fn apply_jitter(seed: u64, i: usize, s: Vec3, use_x: bool, use_y: bool, use_z: bool, p: &mut Vec3) {
    let base = seed ^ (i as u64).wrapping_mul(INDEX_MUL);
    if use_x { p.x += rand_signed(splitmix64(base ^ AX_X)) * s.x; }
    if use_y { p.y += rand_signed(splitmix64(base ^ AX_Y)) * s.y; }
    if use_z { p.z += rand_signed(splitmix64(base ^ AX_Z)) * s.z; }
}

/// Apply per-point random jitter in object space.
///
/// - `selection`: optional point mask. When `None`, all points are affected.
/// - `scale`: global jitter amplitude.
/// - `axis_scales`: per-axis multiplier (X/Y/Z).
/// - `seed`: deterministic random seed.
pub fn jitter_point_positions(
    positions: &mut [Vec3],
    selection: Option<&ElementGroupMask>,
    scale: f32,
    axis_scales: Vec3,
    seed: u64,
) {
    if positions.is_empty() {
        return;
    }
    if scale.abs() <= f32::EPSILON {
        return;
    }
    if axis_scales.length_squared() <= f32::EPSILON {
        return;
    }

    let s = axis_scales * scale;
    let use_x = s.x != 0.0;
    let use_y = s.y != 0.0;
    let use_z = s.z != 0.0;
    if !(use_x || use_y || use_z) {
        return;
    }
    match selection {
        None => {
            for (i, p) in positions.iter_mut().enumerate() {
                apply_jitter(seed, i, s, use_x, use_y, use_z, p);
            }
        }
        Some(mask) => {
            let ones = mask.count_ones();
            if mask.is_empty() || ones == 0 {
                return;
            }
            let sparse = positions.len() > 256 && ones * 2 < positions.len();
            if sparse {
                for i in mask.iter_ones() {
                    if i >= positions.len() {
                        break;
                    }
                    let p = &mut positions[i];
                    apply_jitter(seed, i, s, use_x, use_y, use_z, p);
                }
            } else {
                for (i, p) in positions.iter_mut().enumerate() {
                    if !mask.get(i) {
                        continue;
                    }
                    apply_jitter(seed, i, s, use_x, use_y, use_z, p);
                }
            }
        }
    }
}
