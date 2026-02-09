//! Basis mapping for external DCC/engine coordinate conventions.

use bevy::math::{Mat3, Mat4, Quat, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BasisId { InternalBevy, Unity }

#[derive(Clone, Copy, Debug)]
pub struct BasisMap { m: Mat3, m4: Mat4 }

impl BasisMap {
    #[inline] pub fn map_v3(self, v: Vec3) -> Vec3 { self.m * v }
    #[inline] pub fn map_q(self, q: Quat) -> Quat { Quat::from_mat3(&(self.m * Mat3::from_quat(q) * self.m)) }
    #[inline] pub fn map_m4(self, x: Mat4) -> Mat4 { self.m4 * x * self.m4 }
}

#[inline]
pub fn map(from: BasisId, to: BasisId) -> Option<BasisMap> {
    if from == to { return None; }
    // Unity (LH, Y-up, Z-fwd) -> Bevy internal (RH, Y-up, Z-fwd): mirror Z.
    let mz3 = Mat3::from_diagonal(Vec3::new(1.0, 1.0, -1.0));
    let mz4 = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
    if matches!((from, to), (BasisId::Unity, BasisId::InternalBevy) | (BasisId::InternalBevy, BasisId::Unity)) { Some(BasisMap { m: mz3, m4: mz4 }) } else { None }
}

