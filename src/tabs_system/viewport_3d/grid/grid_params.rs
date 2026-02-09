// Separate Grid Params Logic
use bevy::prelude::*;

pub struct GridParams {
    pub major_step: f32,
    pub minor_step: f32,
    pub center: Vec3,
    pub half_extent: f32,
    pub draw_minor: bool,
}

pub fn grid_params(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    vp: bevy_egui::egui::Vec2,
    major_target_px: f32,
) -> Option<GridParams> {
    let cam_pos = camera_transform.translation();
    let forward = camera_transform.forward();

    // 1. Calculate Grid Center (Intersection of Camera Forward Ray with Y=0 Plane)
    let center = if forward.y.abs() > 1e-4 {
        let t = -cam_pos.y / forward.y;
        if t > 0.0 {
            // Intersects ground in front of camera
            let p = cam_pos + forward * t;
            // Stabilize center: snap to nearest major step? No, major step depends on center.
            // Just clamp distance to avoid extreme horizon values.
            let max_dist = 200.0; // Reasonable interaction range
            let cam_ground = Vec3::new(cam_pos.x, 0.0, cam_pos.z);
            let dist = p.distance(cam_ground);
            if dist > max_dist {
                cam_ground + (p - cam_ground).normalize() * max_dist
            } else {
                Vec3::new(p.x, 0.0, p.z)
            }
        } else {
            Vec3::new(cam_pos.x, 0.0, cam_pos.z)
        }
    } else {
        Vec3::new(cam_pos.x, 0.0, cam_pos.z)
    };

    // 2. Calculate PPM at the center
    // We used to use world_to_ndc here, but that jitters if center moves.
    // Let's use camera altitude for scale if looking down, or distance if looking forward.
    // Actually, PPM at 'center' is the most correct for visual density at the focus point.
    // The user's complaint "hulai huqu" (jittering) is likely because 'nice_step' had too many breakpoints (1,2,5,10).
    // Houdini's progression 1 -> 5 -> 25 -> 125 is geometric with factor 5.

    let ndc0 = camera.world_to_ndc(camera_transform, center);
    let reference_point = if ndc0.is_some() {
        center
    } else {
        Vec3::new(cam_pos.x, 0.0, cam_pos.z)
    };

    let ndc0 = camera.world_to_ndc(camera_transform, reference_point)?;
    let ndc1 = camera.world_to_ndc(camera_transform, reference_point + Vec3::X)?;

    let px0 = Vec2::new((ndc0.x + 1.0) * 0.5 * vp.x, (1.0 - ndc0.y) * 0.5 * vp.y);
    let px1 = Vec2::new((ndc1.x + 1.0) * 0.5 * vp.x, (1.0 - ndc1.y) * 0.5 * vp.y);

    let ppm = (px1 - px0).length().max(1e-4);

    // 3. Houdini-like Geometric Progression (Base 5)
    // Sequence: ..., 0.2, 1, 5, 25, 125, 625, ...
    // Formula: 5^n
    let raw_step = (major_target_px / ppm).max(1e-6);
    let major_step = houdini_step(raw_step);

    // Minor step is usually 1/5th of major step in Houdini (0,1,2,3,4,5)
    // If major is 5, minor is 1. If major is 25, minor is 5.
    let minor_step = major_step / 5.0;

    // Extent
    let diag_px = vp.length();
    let mut world_coverage = diag_px / ppm;
    if world_coverage > 5000.0 {
        world_coverage = 5000.0;
    }

    let half_extent = (world_coverage * 1.5).max(major_step * 4.0);

    // Houdini draws grid even if dense, but fades it. We just toggle minor.
    let draw_minor = (half_extent / minor_step) <= 300.0; // Allow more density

    Some(GridParams {
        major_step,
        minor_step,
        center,
        half_extent,
        draw_minor,
    })
}

fn houdini_step(x: f32) -> f32 {
    // Progression: 5^n
    // log5(x) = ln(x) / ln(5)
    let log5 = x.ln() / 5.0f32.ln();
    let power = log5.ceil(); // ceil ensures we jump to next larger step early, keeping grid stable
                             // actually round() might be better for "closest", ceil() prefers larger grid (less dense)
                             // Houdini grid stays sparse. Let's use ceil to prefer 5 over 1 if x is 1.1

    // Actually, let's use a hysteresis or just strict power
    // User sequence: 1, 5, 25, 100(??), 125...
    // User said: "25 50 75 100" -> This part is linear 25*n?
    // User said: "125 250" -> Doubling?
    // User said: "0 625" -> 5^4
    // User said: "0 3125" -> 5^5

    // It seems the core backbone is powers of 5: 1, 5, 25, 125, 625, 3125.
    // The "25 50 75 100" might be intermediate labels or minor lines?
    // Let's stick to pure powers of 5 for MAJOR steps to start, as it matches the "5, 25, 125" pattern best.

    5.0f32.powf(power)
}
