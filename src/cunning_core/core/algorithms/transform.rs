use crate::libs::geometry::mesh::{Geometry, Attribute};
use bevy::math::{Quat, Vec3};

pub fn transform_geometry(
    input_geo: &Geometry,
    translate: Vec3,
    rotate_euler_deg: Vec3,
    scale: Vec3,
) -> Geometry {
    // Use fork() to ensure we get a new dirty_id for this modified geometry version
    let mut output_geo = input_geo.fork();

    let rotation = Quat::from_euler(
        bevy::prelude::EulerRot::XYZ,
        rotate_euler_deg.x.to_radians(),
        rotate_euler_deg.y.to_radians(),
        rotate_euler_deg.z.to_radians(),
    );

    // Transform Positions (@P)
    if let Some(positions) = output_geo.get_point_attribute_mut("@P").and_then(|a| a.as_mut_slice::<Vec3>()) {
        for p in positions.iter_mut() {
            *p = rotation * (*p * scale) + translate;
        }
    }

    // Transform Normals (@N)
    if let Some(normals) = output_geo.get_vertex_attribute_mut("@N").and_then(|a| a.as_mut_slice::<Vec3>()) {
        // For normals, we generally want to apply rotation.
        // Handling non-uniform scale correctly requires inverse-transpose, but for now
        // we'll stick to rotation to keep it simple and consistent with basic expectations.
        for n in normals.iter_mut() {
            *n = rotation * *n;
            *n = n.normalize();
        }
    }
    
    // Transform Volumes
    // Construct the transformation matrix
    let transform_mat = bevy::math::Mat4::from_scale_rotation_translation(scale, rotation, translate);
    
    // Iterate and update each volume handle (CoW)
    for volume in output_geo.sdfs.iter_mut() {
        // Combine new transform with existing volume transform
        // New = Transform * Old (Parent * Local)
        let new_transform = transform_mat * volume.transform;
        *volume = volume.clone().with_transform(new_transform);
    }
    
    output_geo
}

pub fn transform_geometry_quat(
    input_geo: &Geometry,
    translate: Vec3,
    rotation: Quat,
    scale: Vec3,
) -> Geometry {
    let mut output_geo = input_geo.fork();
    if let Some(positions) = output_geo.get_point_attribute_mut("@P").and_then(|a| a.as_mut_slice::<Vec3>()) {
        for p in positions.iter_mut() {
            *p = rotation * (*p * scale) + translate;
        }
    }
    if let Some(normals) = output_geo.get_vertex_attribute_mut("@N").and_then(|a| a.as_mut_slice::<Vec3>()) {
        for n in normals.iter_mut() {
            *n = rotation * *n;
            *n = n.normalize();
        }
    }
    let transform_mat = bevy::math::Mat4::from_scale_rotation_translation(scale, rotation, translate);
    for volume in output_geo.sdfs.iter_mut() {
        let new_transform = transform_mat * volume.transform;
        *volume = volume.clone().with_transform(new_transform);
    }
    output_geo
}