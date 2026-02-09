use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::mesh::{Attribute, Geometry};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use bevy::prelude::Vec3;
use cunning_kernel::algorithms::algorithms_editor::voxel as vox;
use std::collections::HashMap;
use std::sync::Arc;
// use crate::libs::algorithms::algorithms_dcc::PagedBuffer;

#[derive(Default, Clone)]
pub struct ColorNode;

impl NodeParameters for ColorNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "group",
                "Group",
                "Inputs",
                ParameterValue::String("".to_string()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "class",
                "Class",
                "Settings",
                ParameterValue::Int(0), // 0=Point, 1=Primitive
                ParameterUIType::Dropdown {
                    choices: vec![("Point".to_string(), 0), ("Primitive".to_string(), 1)],
                },
            ),
            Parameter::new(
                "color_type",
                "Color Type",
                "Settings",
                ParameterValue::Int(0), // 0=Constant, 1=Random, 2=BoundingBox
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Constant".to_string(), 0),
                        ("Random".to_string(), 1),
                        ("Bounding Box".to_string(), 2),
                    ],
                },
            ),
            Parameter::new(
                "color",
                "Color",
                "Settings",
                ParameterValue::Color(Vec3::new(1.0, 0.0, 0.0)),
                ParameterUIType::Color { show_alpha: false },
            ),
            Parameter::new(
                "seed",
                "Seed",
                "Settings",
                ParameterValue::Int(0),
                ParameterUIType::IntSlider { min: 0, max: 100 },
            ),
        ]
    }
}

fn compute_color(input: &Geometry, params: &HashMap<String, ParameterValue>) -> Geometry {
    if let Some(out) = compute_color_voxel_payload(input, params) {
        return out;
    }
    let mut geo = input.clone();

    // Parse Params
    let _group_filter = match params.get("group") {
        Some(ParameterValue::String(s)) => s.clone(),
        _ => "".to_string(),
    };

    let class = match params.get("class") {
        Some(ParameterValue::Int(i)) => *i,
        _ => 0,
    };

    let color_type = match params.get("color_type") {
        Some(ParameterValue::Int(i)) => *i,
        _ => 0,
    };

    let constant_color = match params.get("color") {
        Some(ParameterValue::Color(c)) => *c,
        _ => Vec3::ONE,
    };

    let seed = match params.get("seed") {
        Some(ParameterValue::Int(i)) => *i,
        _ => 0,
    };

    // Logic: @Cd
    match class {
        0 => {
            // Point
            let count = match geo.get_point_position_attribute() {
                Some(attr) => attr.len(),
                None => return geo,
            };

            let mut colors = Vec::with_capacity(count);
            let positions = geo.get_point_position_attribute();

            // Precompute BBox if needed
            let (min, size_inv) = if color_type == 2 {
                let mut min = Vec3::splat(f32::MAX);
                let mut max = Vec3::splat(f32::MIN);
                if let Some(pos_buf) = positions {
                    for i in 0..count {
                        if let Some(p) = pos_buf.get(i) {
                            min = min.min(*p);
                            max = max.max(*p);
                        }
                    }
                }
                let size = max - min;
                let size_inv = Vec3::new(
                    if size.x.abs() < 1e-5 {
                        1.0
                    } else {
                        1.0 / size.x
                    },
                    if size.y.abs() < 1e-5 {
                        1.0
                    } else {
                        1.0 / size.y
                    },
                    if size.z.abs() < 1e-5 {
                        1.0
                    } else {
                        1.0 / size.z
                    },
                );
                (min, size_inv)
            } else {
                (Vec3::ZERO, Vec3::ONE)
            };

            for i in 0..count {
                let c = match color_type {
                    0 => constant_color, // Constant
                    1 => {
                        // Random
                        let s = (i as i32 + seed * 12345) as u32;
                        // Simple hash PCG-like
                        let state = s.wrapping_mul(747796405).wrapping_add(2891336453);
                        let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277803737);
                        let res = (word >> 22) ^ word;

                        let r = (res & 0xFF) as f32 / 255.0;
                        let g = ((res >> 8) & 0xFF) as f32 / 255.0;
                        let b = ((res >> 16) & 0xFF) as f32 / 255.0;
                        Vec3::new(r, g, b)
                    }
                    2 => {
                        // BBox
                        if let Some(pos_buf) = positions {
                            if let Some(p) = pos_buf.get(i) {
                                (*p - min) * size_inv
                            } else {
                                Vec3::ZERO
                            }
                        } else {
                            Vec3::ZERO
                        }
                    }
                    _ => Vec3::ZERO,
                };
                colors.push(c);
            }

            geo.insert_point_attribute("@Cd", Attribute::new(colors));
        }
        1 => {
            // Primitive
            let count = geo.primitives().len();
            let mut colors = Vec::with_capacity(count);

            // For BBox on Primitive, we need centroid
            let positions = geo.get_point_position_attribute();
            let (min, size_inv) = if color_type == 2 {
                // Compute BBox of centroids
                let mut min = Vec3::splat(f32::MAX);
                let mut max = Vec3::splat(f32::MIN);

                if let Some(pos_buf) = positions {
                    for prim in geo.primitives().values() {
                        let mut centroid = Vec3::ZERO;
                        let mut n = 0.0;
                        for &v_idx in prim.vertices() {
                            if let Some(v) = geo.vertices().get(v_idx.into()) {
                                if let Some(dense) = geo.points().get_dense_index(v.point_id.into())
                                {
                                    if let Some(p) = pos_buf.get(dense) {
                                        centroid += *p;
                                        n += 1.0;
                                    }
                                }
                            }
                        }
                        if n > 0.0 {
                            centroid /= n;
                            min = min.min(centroid);
                            max = max.max(centroid);
                        }
                    }
                }
                let size = max - min;
                let size_inv = Vec3::new(
                    if size.x.abs() < 1e-5 {
                        1.0
                    } else {
                        1.0 / size.x
                    },
                    if size.y.abs() < 1e-5 {
                        1.0
                    } else {
                        1.0 / size.y
                    },
                    if size.z.abs() < 1e-5 {
                        1.0
                    } else {
                        1.0 / size.z
                    },
                );
                (min, size_inv)
            } else {
                (Vec3::ZERO, Vec3::ONE)
            };

            for i in 0..count {
                let c = match color_type {
                    0 => constant_color,
                    1 => {
                        // Random
                        let s = (i as i32 + seed * 54321) as u32;
                        let state = s.wrapping_mul(747796405).wrapping_add(2891336453);
                        let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277803737);
                        let res = (word >> 22) ^ word;

                        let r = (res & 0xFF) as f32 / 255.0;
                        let g = ((res >> 8) & 0xFF) as f32 / 255.0;
                        let b = ((res >> 16) & 0xFF) as f32 / 255.0;
                        Vec3::new(r, g, b)
                    }
                    2 => {
                        // BBox (Centroid)
                        let mut centroid = Vec3::ZERO;
                        if let Some(pos_buf) = positions {
                            if let Some(prim) = geo.primitives().values().get(i) {
                                let mut n = 0.0;
                                for &v_idx in prim.vertices() {
                                    if let Some(v) = geo.vertices().get(v_idx.into()) {
                                        if let Some(dense) =
                                            geo.points().get_dense_index(v.point_id.into())
                                        {
                                            if let Some(p) = pos_buf.get(dense) {
                                                centroid += *p;
                                                n += 1.0;
                                            }
                                        }
                                    }
                                }
                                if n > 0.0 {
                                    centroid /= n;
                                }
                            }
                        }
                        (centroid - min) * size_inv
                    }
                    _ => constant_color,
                };
                colors.push(c);
            }
            geo.insert_primitive_attribute("@Cd", Attribute::new(colors));
        }
        _ => {}
    }

    geo
}

#[inline]
fn compute_color_voxel_payload(input: &Geometry, params: &HashMap<String, ParameterValue>) -> Option<Geometry> {
    let vs = input
        .get_detail_attribute(crate::nodes::voxel::voxel_edit::ATTR_VOXEL_SIZE_DETAIL)
        .and_then(|a| a.as_slice::<f32>())
        .and_then(|v| v.first().copied())
        .unwrap_or(0.1)
        .max(0.001);
    let mut grid = crate::nodes::voxel::voxel_edit::read_discrete_payload(input, vs)?;
    if grid.voxels.is_empty() {
        return Some(Geometry::new());
    }

    let class = match params.get("class") { Some(ParameterValue::Int(i)) => *i, _ => 0 };
    let _ = class; // Voxel path colors cells; class is ignored.
    let color_type = match params.get("color_type") { Some(ParameterValue::Int(i)) => *i, _ => 0 };
    let constant_color = match params.get("color") { Some(ParameterValue::Color(c)) => *c, _ => Vec3::ONE };
    let seed = match params.get("seed") { Some(ParameterValue::Int(i)) => *i, _ => 0 };

    let clamp_u8 = |v: f32| -> u8 { (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 };
    let rgba_from_vec3 = |c: Vec3| -> [u8; 4] { [clamp_u8(c.x), clamp_u8(c.y), clamp_u8(c.z), 255] };
    let hash_u32 = |mut x: u32| -> u32 { x ^= x >> 16; x = x.wrapping_mul(0x7feb_352d); x ^= x >> 15; x = x.wrapping_mul(0x846c_a68b); x ^= x >> 16; x };
    let hash_cell = |x: i32, y: i32, z: i32, seed: i32| -> u32 {
        let s = seed as u32;
        hash_u32((x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663) ^ (z as u32).wrapping_mul(83492791) ^ s)
    };

    let (mn, mx) = grid.bounds().unwrap_or_default();
    let size = (mx - mn + bevy::prelude::IVec3::ONE).as_vec3().max(Vec3::ONE);
    let inv = Vec3::new(1.0 / size.x, 1.0 / size.y, 1.0 / size.z);

    for (vox::discrete::VoxelCoord(c), v) in grid.voxels.iter_mut() {
        let rgb = match color_type {
            0 => constant_color,
            1 => {
                let h = hash_cell(c.x, c.y, c.z, seed);
                Vec3::new((h & 255) as f32 / 255.0, ((h >> 8) & 255) as f32 / 255.0, ((h >> 16) & 255) as f32 / 255.0)
            }
            2 => ((c.as_vec3() - mn.as_vec3()) * inv).clamp(Vec3::ZERO, Vec3::ONE),
            _ => constant_color,
        };
        v.color_override = Some(rgba_from_vec3(rgb));
    }

    let mut out = crate::nodes::voxel::voxel_edit::discrete_to_surface_mesh(&grid);
    let prim_n = out.primitives().len();
    if prim_n > 0 {
        out.insert_primitive_attribute(crate::nodes::voxel::voxel_edit::ATTR_VOXEL_SRC_PRIM, Attribute::new(vec![true; prim_n]));
    }
    out.set_detail_attribute(crate::nodes::voxel::voxel_edit::ATTR_VOXEL_SIZE_DETAIL, vec![vs]);
    crate::nodes::voxel::voxel_edit::write_discrete_payload(&mut out, &grid);
    Some(out)
}

impl NodeOp for ColorNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let param_map = params
            .iter()
            .map(|p| (p.name.clone(), p.value.clone()))
            .collect();
        if let Some(input) = mats.first() {
            Arc::new(compute_color(input, &param_map))
        } else {
            Arc::new(Geometry::new())
        }
    }
}

crate::register_node!(
    "Color",
    "Attribute",
    crate::nodes::attribute::color_node::ColorNode
);
