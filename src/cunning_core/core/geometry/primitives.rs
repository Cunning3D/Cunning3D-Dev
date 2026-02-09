use crate::libs::geometry::mesh::*;
use crate::libs::geometry::attrs;
use crate::libs::geometry::ids::PointId;
use bevy::math::{Vec3, UVec3};
use std::collections::HashMap;

pub fn create_sphere(radius: f32, rings: usize, segments: usize) -> Geometry {
    let mut geo = Geometry::new();
    let mut point_ids = Vec::new(); // Store generated point IDs
    let mut positions = Vec::new();
    let mut pid_pos = HashMap::new();
    let mut v_normals = Vec::new();

    // Generate points
    for i in 0..=rings {
        let v = i as f32 / rings as f32;
        let phi = v * std::f32::consts::PI; // 0 to PI

        for j in 0..=segments {
            let u = j as f32 / segments as f32;
            let theta = u * std::f32::consts::PI * 2.0; // 0 to 2PI

            let x = radius * phi.sin() * theta.cos();
            let y = radius * phi.cos(); // Y is up
            let z = radius * phi.sin() * theta.sin();

            positions.push(Vec3::new(x, y, z));
            
            // Add point to Geometry
            let pid = geo.add_point();
            point_ids.push(pid);
            pid_pos.insert(pid, Vec3::new(x, y, z));
        }
    }
    
    // Add @P attribute
    geo.insert_point_attribute(attrs::P, Attribute::new(positions));

    // Generate Quads
    for i in 0..rings {
        for j in 0..segments {
            let p0_idx = i * (segments + 1) + j;
            let p1_idx = p0_idx + 1;
            let p2_idx = (i + 1) * (segments + 1) + j + 1;
            let p3_idx = (i + 1) * (segments + 1) + j;

            let p0 = point_ids[p0_idx];
            let p1 = point_ids[p1_idx];
            let p2 = point_ids[p2_idx];
            let p3 = point_ids[p3_idx];

            // Create vertices referring to these points
            let v0 = geo.add_vertex(p0); v_normals.push(pid_pos.get(&p0).copied().unwrap_or(Vec3::Y).normalize_or_zero());
            let v1 = geo.add_vertex(p1); v_normals.push(pid_pos.get(&p1).copied().unwrap_or(Vec3::Y).normalize_or_zero());
            let v2 = geo.add_vertex(p2); v_normals.push(pid_pos.get(&p2).copied().unwrap_or(Vec3::Y).normalize_or_zero());
            let v3 = geo.add_vertex(p3); v_normals.push(pid_pos.get(&p3).copied().unwrap_or(Vec3::Y).normalize_or_zero());

            geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim {
                vertices: vec![v0, v1, v2, v3]
            }));
        }
    }
    
    // Calculate normals (Flat for now, easy to debug)
    geo.calculate_flat_normals(); 
    geo.insert_vertex_attribute(attrs::N, Attribute::new(v_normals));
    
    geo
}

pub fn create_cube(size: f32, divisions: UVec3) -> Geometry {
    let mut geo = Geometry::new();
    let mut point_map = HashMap::new();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    
    // We can't batch push normals easily because add_vertex is called inside the loop.
    // We will build a temporary normal buffer and assign it later, 
    // BUT since we don't have easy vertex index access (VertexId is opaque), 
    // we should rely on the attribute logic or collect VertexIds.
    // Let's collect normals parallel to Vertex creation.

    let half_size = size * 0.5;
    let divs = divisions.max(UVec3::ONE);

    // Define the 6 faces of the cube
    let faces = [
        (Vec3::X, Vec3::Y, Vec3::Z, divs.y, divs.z),   // Right
        (-Vec3::X, Vec3::Y, -Vec3::Z, divs.y, divs.z), // Left
        (Vec3::Y, Vec3::X, -Vec3::Z, divs.x, divs.z),  // Top
        (-Vec3::Y, Vec3::X, Vec3::Z, divs.x, divs.z),  // Bottom
        (Vec3::Z, Vec3::Y, -Vec3::X, divs.x, divs.y),  // Front
        (-Vec3::Z, Vec3::Y, Vec3::X, divs.x, divs.y),  // Back
    ];

    for (normal, u_axis, v_axis, u_divs, v_divs) in faces.iter().cloned() {
        let mut face_point_ids = vec![vec![PointId::INVALID; (v_divs + 1) as usize]; (u_divs + 1) as usize];

        // Generate unique points for this face's grid
        for i in 0..=u_divs {
            for j in 0..=v_divs {
                let u = i as f32 / u_divs as f32;
                let v = j as f32 / v_divs as f32;

                let p = normal * half_size
                    + u_axis * (u - 0.5) * size
                    + v_axis * (v - 0.5) * size;

                // Discretize position to use as a hash map key
                let pos_key = [
                    (p.x * 10000.0) as i32,
                    (p.y * 10000.0) as i32,
                    (p.z * 10000.0) as i32,
                ];

                let pid = *point_map.entry(pos_key).or_insert_with(|| {
                    positions.push(p);
                    geo.add_point()
                });
                face_point_ids[i as usize][j as usize] = pid;
            }
        }

        // Generate vertices and primitives (quads) for the face
        for i in 0..u_divs {
            for j in 0..v_divs {
                let i_usize = i as usize;
                let j_usize = j as usize;

                let p0 = face_point_ids[i_usize][j_usize];
                let p1 = face_point_ids[i_usize + 1][j_usize];
                let p2 = face_point_ids[i_usize + 1][j_usize + 1];
                let p3 = face_point_ids[i_usize][j_usize + 1];

                // Create vertices
                let v0 = geo.add_vertex(p0); normals.push(normal);
                let v1 = geo.add_vertex(p1); normals.push(normal);
                let v2 = geo.add_vertex(p2); normals.push(normal);
                let v3 = geo.add_vertex(p3); normals.push(normal);

                // Fix winding order for faces where U x V points opposite to Normal
                let indices = if normal.dot(u_axis.cross(v_axis)) < 0.0 {
                    vec![v0, v3, v2, v1]
                } else {
                    vec![v0, v1, v2, v3]
                };

                geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: indices }));
            }
        }
    }

    geo.insert_point_attribute(attrs::P, Attribute::new(positions));
    geo.insert_vertex_attribute(attrs::N, Attribute::new(normals));

    geo
}
