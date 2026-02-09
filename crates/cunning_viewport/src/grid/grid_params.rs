use bevy::prelude::*;

pub struct GridParams {
    pub major_step: f32,
    pub minor_step: f32,
    pub center: Vec3,
    pub half_extent: f32,
    pub draw_minor: bool,
}

pub fn grid_params(camera: &Camera, camera_transform: &GlobalTransform, vp: bevy_egui::egui::Vec2, major_target_px: f32) -> Option<GridParams> {
    let cam_pos = camera_transform.translation();
    let forward = camera_transform.forward();

    let center = if forward.y.abs() > 1e-4 {
        let t = -cam_pos.y / forward.y;
        if t > 0.0 {
            let p = cam_pos + forward * t;
            let max_dist = 200.0;
            let cam_ground = Vec3::new(cam_pos.x, 0.0, cam_pos.z);
            let dist = p.distance(cam_ground);
            if dist > max_dist { cam_ground + (p - cam_ground).normalize() * max_dist } else { Vec3::new(p.x, 0.0, p.z) }
        } else {
            Vec3::new(cam_pos.x, 0.0, cam_pos.z)
        }
    } else {
        Vec3::new(cam_pos.x, 0.0, cam_pos.z)
    };

    let ndc0 = camera.world_to_ndc(camera_transform, center);
    let reference_point = if ndc0.is_some() { center } else { Vec3::new(cam_pos.x, 0.0, cam_pos.z) };

    let ndc0 = camera.world_to_ndc(camera_transform, reference_point)?;
    let ndc1 = camera.world_to_ndc(camera_transform, reference_point + Vec3::X)?;

    let px0 = Vec2::new((ndc0.x + 1.0) * 0.5 * vp.x, (1.0 - ndc0.y) * 0.5 * vp.y);
    let px1 = Vec2::new((ndc1.x + 1.0) * 0.5 * vp.x, (1.0 - ndc1.y) * 0.5 * vp.y);

    let ppm = (px1 - px0).length().max(1e-4);
    let raw_step = (major_target_px / ppm).max(1e-6);
    let major_step = houdini_step(raw_step);
    let minor_step = major_step / 5.0;

    let diag_px = vp.length();
    let mut world_coverage = diag_px / ppm;
    if world_coverage > 5000.0 { world_coverage = 5000.0; }

    let half_extent = (world_coverage * 1.5).max(major_step * 4.0);
    let draw_minor = (half_extent / minor_step) <= 300.0;

    Some(GridParams { major_step, minor_step, center, half_extent, draw_minor })
}

fn houdini_step(x: f32) -> f32 {
    let log5 = x.ln() / 5.0f32.ln();
    let power = log5.ceil();
    5.0f32.powf(power)
}

