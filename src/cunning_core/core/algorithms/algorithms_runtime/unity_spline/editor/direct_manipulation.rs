use bevy::math::{Quat, Vec2, Vec3};

#[derive(Clone, Copy, Debug, Default)]
pub struct DirectManipulationState {
    pub initial_position: Vec3,
    pub initial_rotation: Quat,
    pub initial_mouse: Vec2,
    pub is_dragging: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct DirectManipulationConfig {
    pub snap_to_guide: bool,
    pub snap_to_guide_distance_px: f32,
    pub move_snap: Vec3, // x,z used for plane; y used for normal move
}

impl Default for DirectManipulationConfig {
    fn default() -> Self { Self { snap_to_guide: true, snap_to_guide_distance_px: 7.0, move_snap: Vec3::ONE } }
}

#[derive(Clone, Copy, Debug)]
pub struct Ray { pub origin: Vec3, pub dir: Vec3 }

pub trait DirectManipulationAdapter {
    fn gui_point_to_world_ray(&self, mouse: Vec2) -> Ray;
    fn world_to_gui_point(&self, world: Vec3) -> Vec2;
    fn calc_line_translation(&self, start: Vec2, end: Vec2, origin: Vec3, axis: Vec3) -> f32;
}

#[inline] fn snap_scalar(v: f32, snap: f32) -> f32 { if snap == 0.0 { v } else { (v / snap).round() * snap } }
#[inline] fn snap_vec3(v: Vec3, snap: Vec3) -> Vec3 { Vec3::new(snap_scalar(v.x, snap.x), snap_scalar(v.y, snap.y), snap_scalar(v.z, snap.z)) }

#[inline]
fn ray_plane_intersection(ray: Ray, plane_origin: Vec3, plane_normal: Vec3) -> Option<f32> {
    let denom = plane_normal.dot(ray.dir);
    if denom.abs() < 1e-8 { return None; }
    Some((plane_origin - ray.origin).dot(plane_normal) / denom)
}

#[inline]
fn get_snap_to_guide_data<A: DirectManipulationAdapter>(a: &A, current: Vec3, origin: Vec3, axis: Vec3, mouse: Vec2) -> (Vec3, f32) {
    let proj = (current - origin).project_onto(axis);
    let screen = a.world_to_gui_point(origin + proj);
    (proj, screen.distance(mouse))
}

impl DirectManipulationState {
    #[inline]
    pub fn begin_drag(&mut self, position: Vec3, rotation: Quat, mouse: Vec2) {
        self.initial_position = position;
        self.initial_rotation = rotation;
        self.initial_mouse = mouse;
        self.is_dragging = false;
    }

    #[inline]
    pub fn end_drag(&mut self) { self.is_dragging = false; }

    /// Port of UnityEditor.Splines.DirectManipulation.UpdateDrag core semantics (UI-agnostic).
    /// - `move_on_normal`: equivalent to holding Alt in Unity implementation.
    /// - `incremental_snap_active`: equivalent to EditorSnapSettings.incrementalSnapActive.
    pub fn update_drag<A: DirectManipulationAdapter>(&mut self, a: &A, cfg: DirectManipulationConfig, mouse: Vec2, move_on_normal: bool, incremental_snap_active: bool) -> Vec3 {
        let pos = if move_on_normal { self.move_on_normal(a, cfg, mouse) } else { self.move_on_plane(a, cfg, mouse, incremental_snap_active) };
        self.is_dragging = true;
        pos
    }

    fn move_on_plane<A: DirectManipulationAdapter>(&self, a: &A, cfg: DirectManipulationConfig, mouse: Vec2, snapping: bool) -> Vec3 {
        let ray = a.gui_point_to_world_ray(mouse);
        let plane_normal = self.initial_rotation * Vec3::Y;
        let position = match ray_plane_intersection(ray, self.initial_position, plane_normal) {
            Some(t) if t.is_finite() => ray.origin + ray.dir * t,
            _ => self.initial_position,
        };

        let mut dir = position - self.initial_position;
        let forward = get_snap_to_guide_data(a, position, self.initial_position, self.initial_rotation * Vec3::Z, mouse);
        let right = get_snap_to_guide_data(a, position, self.initial_position, self.initial_rotation * Vec3::X, mouse);

        if !snapping && cfg.snap_to_guide {
            if forward.1 < cfg.snap_to_guide_distance_px || right.1 < cfg.snap_to_guide_distance_px {
                let snap_to_forward = forward.1 < right.1;
                let axis = if snap_to_forward { forward.0 } else { right.0 };
                return self.initial_position + axis;
            }
        }

        if dir.length_squared() == 0.0 { dir = Vec3::Z; }
        let local = self.initial_rotation.inverse() * dir;
        let snapped = snap_vec3(local, Vec3::new(cfg.move_snap.x, 0.0, cfg.move_snap.z));
        self.initial_position + self.initial_rotation * snapped
    }

    fn move_on_normal<A: DirectManipulationAdapter>(&self, a: &A, cfg: DirectManipulationConfig, mouse: Vec2) -> Vec3 {
        let up = self.initial_rotation * Vec3::Y;
        let t = a.calc_line_translation(self.initial_mouse, mouse, self.initial_position, up);
        self.initial_position + up * snap_scalar(t, cfg.move_snap.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug)]
    struct DummyAdapter;

    impl DirectManipulationAdapter for DummyAdapter {
        fn gui_point_to_world_ray(&self, mouse: Vec2) -> Ray { Ray { origin: Vec3::new(mouse.x, 0.0, mouse.y), dir: Vec3::Y } }
        fn world_to_gui_point(&self, world: Vec3) -> Vec2 { Vec2::new(world.x, world.z) }
        fn calc_line_translation(&self, start: Vec2, end: Vec2, _origin: Vec3, _axis: Vec3) -> f32 { end.y - start.y }
    }

    #[test]
    fn snap_to_guide_prefers_nearest_axis() {
        let a = DummyAdapter;
        let cfg = DirectManipulationConfig { snap_to_guide: true, snap_to_guide_distance_px: 1000.0, move_snap: Vec3::ONE };
        let mut s = DirectManipulationState::default();
        s.begin_drag(Vec3::ZERO, Quat::IDENTITY, Vec2::ZERO);

        // Mouse ray hits plane at x=10,z=1 -> closer to X axis than Z axis in screen space.
        let pos = s.update_drag(&a, cfg, Vec2::new(10.0, 1.0), false, false);
        assert_eq!(pos.z, 0.0);
    }

    #[test]
    fn move_on_normal_snaps_scalar() {
        let a = DummyAdapter;
        let cfg = DirectManipulationConfig { snap_to_guide: false, snap_to_guide_distance_px: 0.0, move_snap: Vec3::new(0.0, 0.5, 0.0) };
        let mut s = DirectManipulationState::default();
        s.begin_drag(Vec3::ZERO, Quat::IDENTITY, Vec2::ZERO);
        let pos = s.update_drag(&a, cfg, Vec2::new(0.0, 0.6), true, false);
        assert!((pos.y - 0.5).abs() < 1e-6);
    }
}
