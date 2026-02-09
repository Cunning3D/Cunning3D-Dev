use crate::cunning_core::traits::node_interface::{
    GizmoContext, GizmoDrawBuffer, GizmoPart, GizmoPrimitive, GizmoState,
};
use bevy::prelude::*;

pub struct StandardGizmo;

impl StandardGizmo {
    pub fn draw_translate(
        buffer: &mut GizmoDrawBuffer,
        context: &GizmoContext,
        gizmo_state: &mut GizmoState,
        position: &mut Vec3,
        rotation: Quat,
        node_id: uuid::Uuid,
    ) -> bool {
        let mut changed = false;
        let origin = *position;
        let scale = if context.is_orthographic {
            context.scale_factor * 0.075
        } else {
            (context.cam_pos - origin).length() * 0.075
        };
        let axis_len = 1.1 * scale;
        let plane_offset = 0.35 * scale;
        let plane_size = 0.15 * scale;

        let local_x = rotation * Vec3::X;
        let local_y = rotation * Vec3::Y;
        let local_z = rotation * Vec3::Z;

        let axes = [
            (local_x, Color::srgb(1.0, 0.0, 0.0), GizmoPart::TranslateX),
            (local_y, Color::srgb(0.0, 1.0, 0.0), GizmoPart::TranslateY),
            (local_z, Color::srgb(0.0, 0.0, 1.0), GizmoPart::TranslateZ),
        ];

        let planes = [
            (
                local_z,
                GizmoPart::TranslatePlanarXY,
                rotation * Vec3::new(plane_offset, plane_offset, 0.0),
                Color::srgb(0.0, 0.0, 1.0),
            ),
            (
                local_y,
                GizmoPart::TranslatePlanarXZ,
                rotation * Vec3::new(plane_offset, 0.0, plane_offset),
                Color::srgb(0.0, 1.0, 0.0),
            ),
            (
                local_x,
                GizmoPart::TranslatePlanarYZ,
                rotation * Vec3::new(0.0, plane_offset, plane_offset),
                Color::srgb(1.0, 0.0, 0.0),
            ),
        ];

        let is_active_node = gizmo_state.active_node_id == Some(node_id);

        // 1. Handle Dragging (Logic unchanged)
        if is_active_node && context.mouse_left_pressed {
            if let Some(part) = gizmo_state.active_part {
                if let Some(start_pos) = gizmo_state.drag_start_pos {
                    if let Some(initial_pos) = gizmo_state.initial_transform_pos {
                        // Axis Drag
                        let axis_dir = match part {
                            GizmoPart::TranslateX => local_x,
                            GizmoPart::TranslateY => local_y,
                            GizmoPart::TranslateZ => local_z,
                            _ => Vec3::ZERO,
                        };

                        if axis_dir != Vec3::ZERO {
                            let (_, p_axis) = closest_points_ray_line(
                                context.ray_origin,
                                context.ray_direction,
                                initial_pos,
                                axis_dir,
                            );
                            let delta = p_axis - start_pos;
                            *position = initial_transform_pos(gizmo_state) + delta;
                            changed = true;
                            // Draw guide line
                            buffer.draw_line(
                                origin,
                                origin + axis_dir * axis_len * 2.0,
                                Color::WHITE,
                            );
                        }

                        // Planar Drag
                        let plane_normal = match part {
                            GizmoPart::TranslatePlanarXY => local_z,
                            GizmoPart::TranslatePlanarXZ => local_y,
                            GizmoPart::TranslatePlanarYZ => local_x,
                            _ => Vec3::ZERO,
                        };

                        if plane_normal != Vec3::ZERO {
                            if let Some(dist) = ray_plane_intersection(
                                context.ray_origin,
                                context.ray_direction,
                                initial_pos,
                                plane_normal,
                            ) {
                                let current_hit = context.ray_origin + context.ray_direction * dist;
                                let delta = current_hit - start_pos;
                                *position = initial_transform_pos(gizmo_state) + delta;
                                changed = true;
                            }
                        }
                    }
                }
            }
        } else if is_active_node && context.mouse_left_just_released {
            gizmo_state.active_node_id = None;
            gizmo_state.active_part = None;
        }

        // 2. Hit Test (Determine Hover)
        let is_dragging_something = gizmo_state.active_node_id.is_some();
        let mut hover_part = None;
        let mut hit_pos_for_drag = None;

        if !is_dragging_something {
            let hit_threshold = 0.1 * scale;
            let hit_threshold_sq = hit_threshold * hit_threshold;
            let mut best_hit: Option<(f32, f32, GizmoPart, Vec3)> = None;

            // Check Axes
            for (dir, _, part) in axes {
                let end = origin + dir * axis_len;
                let (dist_sq, t_seg) =
                    distance_sq_ray_segment(context.ray_origin, context.ray_direction, origin, end);

                if dist_sq < hit_threshold_sq {
                    let p_seg = origin + dir * (axis_len * t_seg);
                    let dist_cam = (p_seg - context.ray_origin).length_squared();

                    let is_better = if let Some((best_dist, best_cam, _, _)) = best_hit {
                        if dist_sq < best_dist {
                            true
                        } else if (dist_sq - best_dist).abs() < 1e-5 && dist_cam < best_cam {
                            true
                        } else {
                            false
                        }
                    } else {
                        true
                    };

                    if is_better {
                        let (_, p_axis) = closest_points_ray_line(
                            context.ray_origin,
                            context.ray_direction,
                            origin,
                            dir,
                        );
                        best_hit = Some((dist_sq, dist_cam, part, p_axis));
                    }
                }
            }

            // Check Planes
            for (normal, part, offset, _) in planes {
                let center = origin + offset;
                if let Some(dist) = ray_plane_intersection(
                    context.ray_origin,
                    context.ray_direction,
                    center,
                    normal,
                ) {
                    let hit = context.ray_origin + context.ray_direction * dist;
                    let local_hit_vec = hit - center;
                    let max_dist = plane_size;

                    let inside = match part {
                        GizmoPart::TranslatePlanarXY => {
                            local_hit_vec.dot(local_x).abs() < max_dist
                                && local_hit_vec.dot(local_y).abs() < max_dist
                        }
                        GizmoPart::TranslatePlanarXZ => {
                            local_hit_vec.dot(local_x).abs() < max_dist
                                && local_hit_vec.dot(local_z).abs() < max_dist
                        }
                        GizmoPart::TranslatePlanarYZ => {
                            local_hit_vec.dot(local_y).abs() < max_dist
                                && local_hit_vec.dot(local_z).abs() < max_dist
                        }
                        _ => false,
                    };

                    if inside {
                        let dist_sq = 0.0;
                        let dist_cam = (hit - context.ray_origin).length_squared();

                        let is_better = if let Some((best_dist, best_cam, _, _)) = best_hit {
                            if dist_sq < best_dist {
                                true
                            } else if (dist_sq - best_dist).abs() < 1e-5 && dist_cam < best_cam {
                                true
                            } else {
                                false
                            }
                        } else {
                            true
                        };

                        if is_better {
                            best_hit = Some((dist_sq, dist_cam, part, hit));
                        }
                    }
                }
            }

            if let Some((_, _, part, hit_p)) = best_hit {
                hover_part = Some(part);
                hit_pos_for_drag = Some(hit_p);
            }
        }

        // Handle Click
        if let Some(part) = hover_part {
            if context.mouse_left_just_pressed {
                gizmo_state.active_node_id = Some(node_id);
                gizmo_state.active_part = Some(part);
                gizmo_state.initial_transform_pos = Some(origin);
                gizmo_state.drag_start_pos = hit_pos_for_drag;
            }
        }

        // 3. Draw (SOLID GIZMO)
        // Draw Axes (Cylinder + Cone)
        for (dir, color, part) in axes {
            let mut draw_color = color;

            if is_active_node && gizmo_state.active_part == Some(part) {
                draw_color = Color::srgb(1.0, 1.0, 0.0);
            } else if hover_part == Some(part) {
                draw_color = Color::WHITE;
            }

            // Calculate Arrow Geometry
            // Cylinder (Shaft)
            let cone_height = 0.4 * 0.25 * scale; // Relative to scale. Mesh Cone height is 0.4, we scale it?
                                                  // Wait, Asset Mesh Cone height is 0.4. Asset Cylinder height is 1.0.
                                                  // We want Cylinder to be (axis_len - cone_height) long.
                                                  // We want Cone to be cone_height long.

            // Let's define visual proportions:
            let head_len = 0.25 * scale;
            let shaft_len = axis_len - head_len;
            let shaft_radius = 0.0075 * scale; // Even thinner (half of previous)
            let head_radius = 0.06 * scale;

            // Our assets are normalized: Cylinder H=1.0 R=0.05. Cone H=0.4 R=0.15.
            // Scaling Factors:
            let cyl_scale = Vec3::new(shaft_radius / 0.05, shaft_len, shaft_radius / 0.05);
            let cone_scale = Vec3::new(head_radius / 0.15, head_len / 0.4, head_radius / 0.15);

            // Transforms
            // Cylinder Center
            let shaft_center = origin + dir * (shaft_len * 0.5);
            let rot = Quat::from_rotation_arc(Vec3::Y, dir); // Primitives are Y-up

            buffer.draw_mesh(
                GizmoPrimitive::Cylinder,
                Transform::from_translation(shaft_center)
                    .with_rotation(rot)
                    .with_scale(cyl_scale),
                draw_color,
            );

            // Cone Center
            // Cone Mesh origin is center. Height 0.4. Visual base is -0.2, tip +0.2.
            // We want Base at Shaft End.
            // Shaft End = origin + dir * shaft_len.
            // Cone Base = Cone Center - dir * (head_len * 0.5).
            // So Cone Center = Shaft End + dir * (head_len * 0.5).
            //                = origin + dir * (shaft_len + head_len * 0.5).
            let head_center = origin + dir * (shaft_len + head_len * 0.5);

            buffer.draw_mesh(
                GizmoPrimitive::Cone,
                Transform::from_translation(head_center)
                    .with_rotation(rot)
                    .with_scale(cone_scale),
                draw_color,
            );
        }

        // Draw Planar Handles (Plane)
        for (_, part, offset, color) in planes {
            let center = origin + offset;
            let cc = color.to_srgba();
            let mut draw_color = Color::srgba(cc.red, cc.green, cc.blue, 0.4);

            if is_active_node && gizmo_state.active_part == Some(part) {
                draw_color = Color::srgba(1.0, 1.0, 0.0, 0.5);
            } else if hover_part == Some(part) {
                draw_color = Color::srgba(1.0, 1.0, 0.0, 0.8);
            }

            // Plane Mesh is 1.0x1.0 Y-up.
            let size = plane_size * 2.0; // scale factor

            // Orientation
            // XY Plane -> Normal Z. Plane is Y-up. Rotate X->X? No.
            // Plane Mesh normal is Y.
            // XY Plane normal is Z. We need to rotate Y to Z.
            // XZ Plane normal is Y. Matches Mesh.
            // YZ Plane normal is X. Rotate Y to X.

            let plane_rot = match part {
                GizmoPart::TranslatePlanarXY => {
                    rotation * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)
                }
                GizmoPart::TranslatePlanarXZ => rotation,
                GizmoPart::TranslatePlanarYZ => {
                    rotation * Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2)
                }
                _ => Quat::IDENTITY,
            };

            buffer.draw_mesh(
                GizmoPrimitive::Plane,
                Transform::from_translation(center)
                    .with_rotation(plane_rot)
                    .with_scale(Vec3::splat(size)),
                draw_color,
            );
        }

        changed
    }

    pub fn draw_scale(
        buffer: &mut GizmoDrawBuffer,
        context: &GizmoContext,
        gizmo_state: &mut GizmoState,
        position: Vec3,
        scale_val: &mut Vec3,
        rotation: Quat,
        node_id: uuid::Uuid,
    ) -> bool {
        let mut changed = false;
        let origin = position;
        let base_scale = if context.is_orthographic {
            context.scale_factor * 0.075
        } else {
            (context.cam_pos - origin).length() * 0.075
        };

        let axis_len = 0.7 * base_scale;
        // let box_half = 1.35 * base_scale; // Deprecated: Handles now follow scale
        let handle_size = 0.15 * base_scale;

        let local_x = rotation * Vec3::X;
        let local_y = rotation * Vec3::Y;
        let local_z = rotation * Vec3::Z;

        let axes = [
            (local_x, Color::srgb(1.0, 0.0, 0.0), GizmoPart::ScaleX),
            (local_y, Color::srgb(0.0, 1.0, 0.0), GizmoPart::ScaleY),
            (local_z, Color::srgb(0.0, 0.0, 1.0), GizmoPart::ScaleZ),
        ];

        // For handles (Cubes at ends of axes)
        // Position them on the faces of the scale box (scale_val)
        let sx = scale_val.x.abs();
        let sy = scale_val.y.abs();
        let sz = scale_val.z.abs();

        let faces = [
            (
                local_x,
                GizmoPart::ScaleX,
                local_x * sx,
                Color::srgb(1.0, 0.0, 0.0),
            ),
            (
                -local_x,
                GizmoPart::ScaleX,
                -local_x * sx,
                Color::srgb(1.0, 0.0, 0.0),
            ),
            (
                local_y,
                GizmoPart::ScaleY,
                local_y * sy,
                Color::srgb(0.0, 1.0, 0.0),
            ),
            (
                -local_y,
                GizmoPart::ScaleY,
                -local_y * sy,
                Color::srgb(0.0, 1.0, 0.0),
            ),
            (
                local_z,
                GizmoPart::ScaleZ,
                local_z * sz,
                Color::srgb(0.0, 0.0, 1.0),
            ),
            (
                -local_z,
                GizmoPart::ScaleZ,
                -local_z * sz,
                Color::srgb(0.0, 0.0, 1.0),
            ),
        ];

        let is_active_node = gizmo_state.active_node_id == Some(node_id);

        // 1. Handle Dragging (Logic unchanged)
        if is_active_node && context.mouse_left_pressed {
            if let Some(part) = gizmo_state.active_part {
                if let Some(start_pos) = gizmo_state.drag_start_pos {
                    if let Some(initial_scale) = gizmo_state.initial_transform_pos {
                        let (axis_dir, axis_idx) = match part {
                            GizmoPart::ScaleX => (local_x, 0),
                            GizmoPart::ScaleY => (local_y, 1),
                            GizmoPart::ScaleZ => (local_z, 2),
                            _ => (Vec3::ZERO, 0),
                        };

                        if axis_dir != Vec3::ZERO {
                            let (_, p_axis) = closest_points_ray_line(
                                context.ray_origin,
                                context.ray_direction,
                                origin,
                                axis_dir,
                            );

                            let dist_start = (start_pos - origin).dot(axis_dir);
                            let dist_current = (p_axis - origin).dot(axis_dir);

                            if dist_start.abs() > 0.001 {
                                let ratio = dist_current / dist_start;
                                let mut new_scale = initial_scale;
                                new_scale[axis_idx] *= ratio;
                                *scale_val = new_scale;
                                changed = true;
                            }
                        }
                    }
                }
            }
        } else if is_active_node && context.mouse_left_just_released {
            gizmo_state.active_node_id = None;
            gizmo_state.active_part = None;
        }

        // 2. Hit Test
        let is_dragging_something = gizmo_state.active_node_id.is_some();
        let mut hover_part = None;
        let mut hit_pos_for_drag = None;

        if !is_dragging_something {
            let hit_threshold_faces = handle_size * 1.5;
            let hit_threshold_sq_faces = hit_threshold_faces * hit_threshold_faces;

            let hit_threshold_axes = 0.1 * base_scale;
            let hit_threshold_sq_axes = hit_threshold_axes * hit_threshold_axes;

            let mut best_hit: Option<(f32, f32, GizmoPart, Vec3)> = None;

            // Check Axes
            for (dir, _, part) in axes {
                let end = origin + dir * axis_len;
                let (dist_sq, t_seg) =
                    distance_sq_ray_segment(context.ray_origin, context.ray_direction, origin, end);

                if dist_sq < hit_threshold_sq_axes {
                    let p_seg = origin + dir * (axis_len * t_seg);
                    let dist_cam = (p_seg - context.ray_origin).length_squared();

                    let is_better = if let Some((best_dist, best_cam, _, _)) = best_hit {
                        if dist_sq < best_dist {
                            true
                        } else if (dist_sq - best_dist).abs() < 1e-5 && dist_cam < best_cam {
                            true
                        } else {
                            false
                        }
                    } else {
                        true
                    };

                    if is_better {
                        let (_, p_axis) = closest_points_ray_line(
                            context.ray_origin,
                            context.ray_direction,
                            origin,
                            dir,
                        );
                        best_hit = Some((dist_sq, dist_cam, part, p_axis));
                    }
                }
            }

            for (_, part, center_offset, _) in faces {
                let face_center = origin + center_offset;
                let (dist_sq, _) =
                    distance_sq_ray_point(context.ray_origin, context.ray_direction, face_center);

                if dist_sq < hit_threshold_sq_faces {
                    let dist_cam = (face_center - context.ray_origin).length_squared();
                    let is_better = if let Some((best_dist, best_cam, _, _)) = best_hit {
                        if dist_sq < best_dist {
                            true
                        } else if (dist_sq - best_dist).abs() < 1e-5 && dist_cam < best_cam {
                            true
                        } else {
                            false
                        }
                    } else {
                        true
                    };

                    if is_better {
                        best_hit = Some((dist_sq, dist_cam, part, face_center));
                    }
                }
            }

            if let Some((_, _, part, hit_p)) = best_hit {
                hover_part = Some(part);
                hit_pos_for_drag = Some(hit_p);
            }
        }

        // Handle Click
        if let Some(part) = hover_part {
            if context.mouse_left_just_pressed {
                gizmo_state.active_node_id = Some(node_id);
                gizmo_state.active_part = Some(part);
                gizmo_state.initial_transform_pos = Some(*scale_val);
                gizmo_state.drag_start_pos = hit_pos_for_drag;
            }
        }

        // 3. Draw (SOLID)

        // Draw Scale Axes (Shafts only) - REMOVED to avoid Z-fighting with Translate Gizmo shafts
        // Since Translate Gizmo is always drawn and its shafts are longer, they serve as the visual support for Scale handles.

        // Faces (Cubes)
        for (_, part, center_offset, color) in faces {
            let face_center = origin + center_offset;
            let mut draw_color = color;

            if is_active_node && gizmo_state.active_part == Some(part) {
                draw_color = Color::srgb(1.0, 1.0, 0.0);
            } else if hover_part == Some(part) {
                draw_color = Color::WHITE;
            }

            buffer.draw_mesh(
                GizmoPrimitive::Cube,
                Transform::from_translation(face_center)
                    .with_rotation(rotation)
                    .with_scale(Vec3::splat(handle_size)),
                draw_color,
            );
        }

        // Wireframe Box (Restored using Lines)
        let box_transform = Transform::from_translation(origin)
            .with_rotation(rotation)
            .with_scale(*scale_val * 2.0);
        draw_wire_box(buffer, box_transform, Color::srgb(0.5, 0.5, 0.5));

        changed
    }

    pub fn draw_rotate(
        buffer: &mut GizmoDrawBuffer,
        context: &GizmoContext,
        gizmo_state: &mut GizmoState,
        position: Vec3,
        rotation: &mut Vec3, // Euler degrees
        node_id: uuid::Uuid,
    ) -> bool {
        let mut changed = false;
        let origin = position;
        let base_scale = if context.is_orthographic {
            context.scale_factor * 0.075
        } else {
            (context.cam_pos - origin).length() * 0.075
        };
        let radius = 1.2 * base_scale;

        // Convert Euler (YXZ) to Quat to get local axes
        let rot_quat = Quat::from_euler(
            EulerRot::YXZ,
            rotation.y.to_radians(),
            rotation.x.to_radians(),
            rotation.z.to_radians(),
        );
        let local_x = rot_quat * Vec3::X;
        let local_y = rot_quat * Vec3::Y;
        let local_z = rot_quat * Vec3::Z;

        let axes = [
            (
                local_x,
                Color::srgb(1.0, 0.0, 0.0),
                GizmoPart::RotateX,
                local_y,
                local_z,
            ),
            (
                local_y,
                Color::srgb(0.0, 1.0, 0.0),
                GizmoPart::RotateY,
                local_z,
                local_x,
            ),
            (
                local_z,
                Color::srgb(0.0, 0.0, 1.0),
                GizmoPart::RotateZ,
                local_x,
                local_y,
            ),
        ];

        let is_active_node = gizmo_state.active_node_id == Some(node_id);

        // 1. Handle Dragging (Logic unchanged)
        if is_active_node && context.mouse_left_pressed {
            if let Some(part) = gizmo_state.active_part {
                if let Some(_start_ray_dir) = gizmo_state.drag_start_ray.map(|(_, d)| d) {
                    if let Some(initial_rot) = gizmo_state.initial_transform_pos {
                        // Handle Axis Rotation
                        let (axis_vec, axis_idx) = match part {
                            GizmoPart::RotateX => (local_x, 0),
                            GizmoPart::RotateY => (local_y, 1),
                            GizmoPart::RotateZ => (local_z, 2),
                            _ => (Vec3::ZERO, 0),
                        };

                        if axis_vec != Vec3::ZERO {
                            let plane_normal = axis_vec;
                            if let Some(dist) = ray_plane_intersection(
                                context.ray_origin,
                                context.ray_direction,
                                origin,
                                plane_normal,
                            ) {
                                let hit_point = context.ray_origin + context.ray_direction * dist;
                                let current_vec = (hit_point - origin).normalize_or_zero();

                                if let Some(start_hit) = gizmo_state.drag_start_pos {
                                    let start_vec = (start_hit - origin).normalize_or_zero();

                                    let angle = start_vec.angle_between(current_vec);
                                    let cross = start_vec.cross(current_vec);
                                    let sign = cross.dot(plane_normal).signum();

                                    let delta_deg = angle.to_degrees() * sign;

                                    let mut new_rot = initial_rot;
                                    new_rot[axis_idx] += delta_deg;
                                    *rotation = new_rot;
                                    changed = true;

                                    buffer.draw_line(origin, hit_point, Color::WHITE);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Screen Rotate Drag Logic
        if is_active_node
            && context.mouse_left_pressed
            && gizmo_state.active_part == Some(GizmoPart::RotateScreen)
        {
            if let Some(initial_rot) = gizmo_state.initial_transform_pos {
                if let Some(start_hit) = gizmo_state.drag_start_pos {
                    let view_dir = (context.ray_origin - origin).normalize_or_zero();
                    let screen_normal = view_dir;

                    if let Some(dist) = ray_plane_intersection(
                        context.ray_origin,
                        context.ray_direction,
                        origin,
                        screen_normal,
                    ) {
                        let current_hit = context.ray_origin + context.ray_direction * dist;
                        let v_start = (start_hit - origin).normalize_or_zero();
                        let v_curr = (current_hit - origin).normalize_or_zero();

                        let dot = v_start.dot(v_curr).clamp(-1.0, 1.0);
                        let angle = dot.acos();
                        let cross = v_start.cross(v_curr);
                        let sign = cross.dot(screen_normal).signum();
                        let delta_rad = angle * sign;

                        let q_initial = Quat::from_euler(
                            EulerRot::YXZ,
                            initial_rot.y.to_radians(),
                            initial_rot.x.to_radians(),
                            initial_rot.z.to_radians(),
                        );
                        let q_delta = Quat::from_axis_angle(view_dir, delta_rad);
                        let q_new = q_delta * q_initial;
                        let (y, x, z) = q_new.to_euler(EulerRot::YXZ);
                        *rotation = Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees());
                        changed = true;
                    }
                }
            }
        } else if is_active_node && context.mouse_left_just_released {
            gizmo_state.active_node_id = None;
            gizmo_state.active_part = None;
        }

        // 2. Hit Test (Logic unchanged)
        let is_dragging_something = gizmo_state.active_node_id.is_some();
        let mut hover_part = None;
        let mut hit_pos_for_drag = None;

        if !is_dragging_something {
            let hit_threshold = 0.1 * base_scale;
            let mut best_hit: Option<(f32, f32, GizmoPart, Vec3)> = None;

            // Axes Rings
            for (normal, _, part, _, _) in axes {
                if let Some(dist) = ray_plane_intersection(
                    context.ray_origin,
                    context.ray_direction,
                    origin,
                    normal,
                ) {
                    let hit_point = context.ray_origin + context.ray_direction * dist;
                    let dist_from_center = (hit_point - origin).length();
                    let dist_diff = (dist_from_center - radius).abs();

                    if dist_diff < hit_threshold {
                        let dist_cam = (hit_point - context.ray_origin).length_squared();
                        let is_better = if let Some((best_dist, best_cam, _, _)) = best_hit {
                            if dist_diff < best_dist {
                                true
                            } else if (dist_diff - best_dist).abs() < 1e-5 && dist_cam < best_cam {
                                true
                            } else {
                                false
                            }
                        } else {
                            true
                        };

                        if is_better {
                            best_hit = Some((dist_diff, dist_cam, part, hit_point));
                        }
                    }
                }
            }

            // Screen Ring
            let view_dir = (context.ray_origin - origin).normalize_or_zero();
            let screen_radius = 1.35 * base_scale;
            if let Some(dist) =
                ray_plane_intersection(context.ray_origin, context.ray_direction, origin, view_dir)
            {
                let hit_point = context.ray_origin + context.ray_direction * dist;
                let dist_from_center = (hit_point - origin).length();
                let dist_diff = (dist_from_center - screen_radius).abs();

                if dist_diff < hit_threshold {
                    let dist_cam = (hit_point - context.ray_origin).length_squared();
                    let is_better = if let Some((best_dist, best_cam, _, _)) = best_hit {
                        if dist_diff < best_dist {
                            true
                        } else if (dist_diff - best_dist).abs() < 1e-5 && dist_cam < best_cam {
                            true
                        } else {
                            false
                        }
                    } else {
                        true
                    };

                    if is_better {
                        best_hit = Some((dist_diff, dist_cam, GizmoPart::RotateScreen, hit_point));
                    }
                }
            }

            if let Some((_, _, part, hit_p)) = best_hit {
                hover_part = Some(part);
                hit_pos_for_drag = Some(hit_p);
            }
        }

        // Handle Click
        if let Some(part) = hover_part {
            if context.mouse_left_just_pressed {
                gizmo_state.active_node_id = Some(node_id);
                gizmo_state.active_part = Some(part);
                gizmo_state.initial_transform_pos = Some(*rotation);
                gizmo_state.drag_start_pos = hit_pos_for_drag;
                gizmo_state.drag_start_ray = Some((context.ray_origin, context.ray_direction));
            }
        }

        // 3. Draw (LINES for Rotation)
        for (_, color, part, u_axis, v_axis) in axes {
            let mut draw_color = color;
            if is_active_node && gizmo_state.active_part == Some(part) {
                draw_color = Color::srgb(1.0, 1.0, 0.0);
            } else if hover_part == Some(part) {
                draw_color = Color::WHITE;
            }

            let segments = 64;
            for i in 0..segments {
                let angle1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
                let angle2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
                let p1 = origin + (u_axis * angle1.cos() + v_axis * angle1.sin()) * radius;
                let p2 = origin + (u_axis * angle2.cos() + v_axis * angle2.sin()) * radius;
                buffer.draw_line(p1, p2, draw_color);
            }
        }

        // Screen Ring
        // Planar Billboarding using Camera Rotation (Stable)
        let screen_radius = 1.35 * base_scale;
        let screen_u = context.cam_rotation * Vec3::X;
        let screen_v = context.cam_rotation * Vec3::Y;

        let mut screen_color = Color::WHITE;
        if is_active_node && gizmo_state.active_part == Some(GizmoPart::RotateScreen) {
            screen_color = Color::srgb(1.0, 1.0, 0.0);
        } else if hover_part == Some(GizmoPart::RotateScreen) {
            screen_color = Color::srgba(1.0, 1.0, 0.0, 0.8);
        }

        let segments = 64;
        for i in 0..segments {
            let angle1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
            let angle2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
            let p1 = origin + (screen_u * angle1.cos() + screen_v * angle1.sin()) * screen_radius;
            let p2 = origin + (screen_u * angle2.cos() + screen_v * angle2.sin()) * screen_radius;
            buffer.draw_line(p1, p2, screen_color);
        }

        changed
    }

    pub fn draw_status_hud(ui: &mut bevy_egui::egui::Ui, gizmo_state: &GizmoState) {
        if let Some(part) = gizmo_state.active_part {
            let text = match part {
                GizmoPart::TranslateX => "Moving X",
                GizmoPart::TranslateY => "Moving Y",
                GizmoPart::TranslateZ => "Moving Z",
                GizmoPart::TranslatePlanarXY => "Moving XY",
                GizmoPart::TranslatePlanarXZ => "Moving XZ",
                GizmoPart::TranslatePlanarYZ => "Moving YZ",
                GizmoPart::ScaleX => "Scaling X",
                GizmoPart::ScaleY => "Scaling Y",
                GizmoPart::ScaleZ => "Scaling Z",
                GizmoPart::RotateX => "Rotating X",
                GizmoPart::RotateY => "Rotating Y",
                GizmoPart::RotateZ => "Rotating Z",
                GizmoPart::RotateScreen => "Rotating Screen",
                _ => "Transforming",
            };

            let rect = ui.available_rect_before_wrap();
            let overlay_pos = rect.min + bevy_egui::egui::vec2(10.0, rect.height() - 30.0);

            ui.put(
                bevy_egui::egui::Rect::from_min_size(
                    overlay_pos,
                    bevy_egui::egui::vec2(200.0, 20.0),
                ),
                bevy_egui::egui::Label::new(
                    bevy_egui::egui::RichText::new(text)
                        .strong()
                        .color(bevy_egui::egui::Color32::YELLOW)
                        .background_color(bevy_egui::egui::Color32::from_black_alpha(128)),
                ),
            );
        }
    }
}

fn initial_transform_pos(state: &GizmoState) -> Vec3 {
    state.initial_transform_pos.unwrap_or(Vec3::ZERO)
}

// --- Math Helpers (unchanged) ---

fn closest_points_ray_line(
    ray_origin: Vec3,
    ray_dir: Vec3,
    line_origin: Vec3,
    line_dir: Vec3,
) -> (Vec3, Vec3) {
    let w0 = ray_origin - line_origin;
    let a = ray_dir.dot(ray_dir);
    let b = ray_dir.dot(line_dir);
    let c = line_dir.dot(line_dir);
    let d = ray_dir.dot(w0);
    let e = line_dir.dot(w0);

    let denom = a * c - b * b;

    let (sc, tc);
    if denom < 1e-5 {
        sc = 0.0;
        tc = if b > c { d / b } else { e / c };
    } else {
        sc = (b * e - c * d) / denom;
        tc = (a * e - b * d) / denom;
    }

    (ray_origin + ray_dir * sc, line_origin + line_dir * tc)
}

fn distance_sq_ray_segment(ray_origin: Vec3, ray_dir: Vec3, p0: Vec3, p1: Vec3) -> (f32, f32) {
    let seg_dir = p1 - p0;
    let seg_len_sq = seg_dir.length_squared();
    if seg_len_sq < 1e-5 {
        return ((ray_origin - p0).length_squared(), 0.0);
    }

    let (_, p_line) = closest_points_ray_line(ray_origin, ray_dir, p0, seg_dir);
    let t = (p_line - p0).dot(seg_dir) / seg_len_sq;
    let t_clamped = t.clamp(0.0, 1.0);
    let p_seg = p0 + seg_dir * t_clamped;
    let (dist_sq, _) = distance_sq_ray_point(ray_origin, ray_dir, p_seg);
    (dist_sq, t_clamped)
}

fn distance_sq_ray_point(ray_origin: Vec3, ray_dir: Vec3, point: Vec3) -> (f32, Vec3) {
    let w = point - ray_origin;
    let proj = w.dot(ray_dir);
    if proj < 0.0 {
        return ((ray_origin - point).length_squared(), ray_origin);
    }
    let closest = ray_origin + ray_dir * proj;
    ((closest - point).length_squared(), closest)
}

fn ray_plane_intersection(
    ray_origin: Vec3,
    ray_dir: Vec3,
    plane_origin: Vec3,
    plane_normal: Vec3,
) -> Option<f32> {
    let denom = plane_normal.dot(ray_dir);
    if denom.abs() < 1e-5 {
        return None;
    }
    let t = (plane_origin - ray_origin).dot(plane_normal) / denom;
    if t >= 0.0 {
        Some(t)
    } else {
        None
    }
}

fn draw_wire_box(buffer: &mut GizmoDrawBuffer, transform: Transform, color: Color) {
    // Bevy gizmos.cuboid takes Transform where scale is full size.
    // But here we are drawing lines manually relative to center.
    // The transform already contains the scale.
    // We just need to draw a unit cube transformed by it.
    // Corners of a unit cube (size 1.0) range from -0.5 to 0.5.

    let corners = [
        Vec3::new(-0.5, -0.5, -0.5),
        Vec3::new(0.5, -0.5, -0.5),
        Vec3::new(0.5, 0.5, -0.5),
        Vec3::new(-0.5, 0.5, -0.5),
        Vec3::new(-0.5, -0.5, 0.5),
        Vec3::new(0.5, -0.5, 0.5),
        Vec3::new(0.5, 0.5, 0.5),
        Vec3::new(-0.5, 0.5, 0.5),
    ];

    let world_corners: Vec<Vec3> = corners
        .iter()
        .map(|&c| transform.transform_point(c))
        .collect();

    let edges = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0), // Back
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4), // Front
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7), // Connecting
    ];

    for (i, j) in edges {
        buffer.draw_line(world_corners[i], world_corners[j], color);
    }
}
