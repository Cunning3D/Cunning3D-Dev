use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::algorithms::algorithms_runtime::resample::{
    resample_bezier_knots, ResamplePolicy,
};
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::group::ElementGroupMask;
use crate::libs::geometry::ids::{AttributeId, HalfEdgeId, PointId, VertexId};
use crate::mesh::{Attribute, GeoPrimitive, Geometry, PolygonPrim, PolylinePrim};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Default)]
pub struct PolyExtrudeNode;

impl NodeParameters for PolyExtrudeNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "group",
                "Group",
                "General",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "split",
                "Divide Into",
                "General",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Connected Components".into(), 0),
                        ("Individual Elements".into(), 1),
                    ],
                },
            ),
            Parameter::new(
                "distance",
                "Distance",
                "Extrusion",
                ParameterValue::Float(0.0),
                ParameterUIType::FloatSlider {
                    min: -10.0,
                    max: 10.0,
                },
            ),
            Parameter::new(
                "inset",
                "Inset",
                "Extrusion",
                ParameterValue::Float(0.0),
                ParameterUIType::FloatSlider { min: 0.0, max: 5.0 },
            ),
            Parameter::new(
                "divisions",
                "Divisions",
                "Extrusion",
                ParameterValue::Int(1),
                ParameterUIType::IntSlider { min: 1, max: 10 },
            ),
            // Output
            Parameter::new(
                "output_front",
                "Output Front",
                "Output",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "output_back",
                "Output Back",
                "Output",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "output_side",
                "Output Side",
                "Output",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "front_grp",
                "Front Group",
                "Groups",
                ParameterValue::String("extrudeFront".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "back_grp",
                "Back Group",
                "Groups",
                ParameterValue::String("extrudeBack".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "side_grp",
                "Side Group",
                "Groups",
                ParameterValue::String("extrudeSide".into()),
                ParameterUIType::String,
            ),
        ]
    }
}

impl NodeOp for PolyExtrudeNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let default_geo = Arc::new(Geometry::new());
        let input_geo = mats.first().unwrap_or(&default_geo);
        let mut curve_work: Option<Geometry> = None;
        let input_geo = if input_geo
            .primitives()
            .iter()
            .any(|p| matches!(p, GeoPrimitive::BezierCurve(_)))
        {
            curve_work = Some(curves_to_polygons_minimal(
                input_geo,
                ResamplePolicy::default(),
            ));
            curve_work.as_ref().unwrap()
        } else {
            input_geo
        };

        let group_str = get_param_string(params, "group", "");
        let split_mode = get_param_int(params, "split", 0); // 0: Connected, 1: Individual
        let distance = get_param_float(params, "distance", 0.0);
        let inset = get_param_float(params, "inset", 0.0);
        let divisions = get_param_int(params, "divisions", 1).max(1) as u32;

        let out_front = get_param_bool(params, "output_front", true);
        let out_back = get_param_bool(params, "output_back", false);
        let out_side = get_param_bool(params, "output_side", true);

        let front_grp_name = get_param_string(params, "front_grp", "extrudeFront");
        let back_grp_name = get_param_string(params, "back_grp", "extrudeBack");
        let side_grp_name = get_param_string(params, "side_grp", "extrudeSide");

        let prim_count = input_geo.primitives().len();
        let selection = if group_str.is_empty() {
            let mut mask = ElementGroupMask::new(prim_count);
            mask.invert();
            mask
        } else {
            input_geo
                .primitive_groups
                .get(&AttributeId::from(group_str.as_str()))
                .cloned()
                .unwrap_or_else(|| {
                    crate::nodes::group::utils::parse_pattern(&group_str, prim_count)
                })
        };

        let Some(in_pos) = input_geo.get_point_position_attribute() else {
            return Arc::new(Geometry::new());
        };
        let in_normals = input_geo
            .get_point_attribute(attrs::N)
            .and_then(|a| a.as_slice::<Vec3>());
        let topo = input_geo.get_topology();

        let mut out_geo = Geometry::new();
        let mut out_positions: Vec<Vec3> = Vec::new();

        let mut base_map: HashMap<PointId, PointId> = HashMap::new();
        let mut front_map: HashMap<PointId, PointId> = HashMap::new();

        let mut grp_front = ElementGroupMask::new(0);
        let mut grp_back = ElementGroupMask::new(0);
        let mut grp_side = ElementGroupMask::new(0);

        let alloc_point =
            |pos: Vec3, out_geo: &mut Geometry, out_positions: &mut Vec<Vec3>| -> PointId {
                let id = PointId::from(out_geo.points_mut().insert(()));
                out_positions.push(pos);
                id
            };

        let get_in_pos = |pid: PointId| -> Option<Vec3> {
            let di = input_geo.points().get_dense_index(pid.into())?;
            Some(*in_pos.get(di)?)
        };

        let get_or_create_base = |pid: PointId,
                                  out_geo: &mut Geometry,
                                  out_positions: &mut Vec<Vec3>,
                                  base_map: &mut HashMap<PointId, PointId>|
         -> Option<PointId> {
            if let Some(&np) = base_map.get(&pid) {
                return Some(np);
            }
            let pos = get_in_pos(pid)?;
            let np = alloc_point(pos, out_geo, out_positions);
            base_map.insert(pid, np);
            Some(np)
        };

        if split_mode == 1 {
            // Individual Elements: each selected Polygon is extruded independently (no shared points)
            for (prim_idx, prim) in input_geo.primitives().iter().enumerate() {
                if !selection.get(prim_idx) {
                    continue;
                }
                let GeoPrimitive::Polygon(poly) = prim else {
                    continue;
                };
                if poly.vertices.len() < 3 {
                    continue;
                }

                let mut pts = Vec::with_capacity(poly.vertices.len());
                for &vid in &poly.vertices {
                    let Some(v) = input_geo.vertices().get(vid.into()) else {
                        pts.clear();
                        break;
                    };
                    let Some(p) = get_in_pos(v.point_id) else {
                        pts.clear();
                        break;
                    };
                    pts.push((v.point_id, p));
                }
                if pts.len() < 3 {
                    continue;
                }

                let mut center = Vec3::ZERO;
                for (_, p) in &pts {
                    center += *p;
                }
                center /= pts.len() as f32;
                let normal = (pts[1].1 - pts[0].1)
                    .cross(pts[2].1 - pts[0].1)
                    .normalize_or_zero();

                let mut front_verts: Vec<VertexId> = Vec::with_capacity(pts.len());
                let mut base_verts: Vec<VertexId> = Vec::with_capacity(pts.len());

                for (_, p) in &pts {
                    let offset_dir = if inset != 0.0 {
                        (*p - center).normalize_or_zero() * -inset
                    } else {
                        Vec3::ZERO
                    };
                    let p_front = alloc_point(
                        *p + normal * distance + offset_dir,
                        &mut out_geo,
                        &mut out_positions,
                    );
                    let p_base = alloc_point(*p, &mut out_geo, &mut out_positions);
                    front_verts.push(VertexId::from(
                        out_geo
                            .vertices_mut()
                            .insert(crate::mesh::GeoVertex { point_id: p_front }),
                    ));
                    base_verts.push(VertexId::from(
                        out_geo
                            .vertices_mut()
                            .insert(crate::mesh::GeoVertex { point_id: p_base }),
                    ));
                }

                if out_front {
                    let _ = out_geo
                        .primitives_mut()
                        .insert(GeoPrimitive::Polygon(PolygonPrim {
                            vertices: front_verts.clone(),
                        }));
                    grp_front.push(true);
                    grp_back.push(false);
                    grp_side.push(false);
                }

                if out_back {
                    base_verts.reverse();
                    let _ = out_geo
                        .primitives_mut()
                        .insert(GeoPrimitive::Polygon(PolygonPrim {
                            vertices: base_verts.clone(),
                        }));
                    grp_front.push(false);
                    grp_back.push(true);
                    grp_side.push(false);
                }

                if out_side {
                    let n = base_verts.len();
                    for i in 0..n {
                        let next = (i + 1) % n;
                        // Winding: base(i)->base(next)->front(next)->front(i) to keep outward normals consistent.
                        let quad = vec![
                            base_verts[i],
                            base_verts[next],
                            front_verts[next],
                            front_verts[i],
                        ];
                        let _ = out_geo
                            .primitives_mut()
                            .insert(GeoPrimitive::Polygon(PolygonPrim { vertices: quad }));
                        grp_front.push(false);
                        grp_back.push(false);
                        grp_side.push(true);
                    }
                }
            }
        } else {
            // Connected Components: build shared front points for all selected points
            let mut selected_points: HashSet<PointId> = HashSet::new();
            for (prim_idx, prim) in input_geo.primitives().iter().enumerate() {
                if !selection.get(prim_idx) {
                    continue;
                }
                match prim {
                    GeoPrimitive::Polygon(p) => {
                        for &vid in &p.vertices {
                            if let Some(v) = input_geo.vertices().get(vid.into()) {
                                selected_points.insert(v.point_id);
                            }
                        }
                    }
                    GeoPrimitive::Polyline(p) => {
                        for &vid in &p.vertices {
                            if let Some(v) = input_geo.vertices().get(vid.into()) {
                                selected_points.insert(v.point_id);
                            }
                        }
                    }
                    _ => {}
                };
            }

            // accumulate normals
            let mut n_sum: HashMap<PointId, Vec3> = HashMap::new();
            let mut n_cnt: HashMap<PointId, f32> = HashMap::new();

            for (prim_idx, prim) in input_geo.primitives().iter().enumerate() {
                if !selection.get(prim_idx) {
                    continue;
                }
                match prim {
                    GeoPrimitive::Polygon(poly) => {
                        if poly.vertices.len() < 3 {
                            continue;
                        }
                        let mut pids = Vec::with_capacity(poly.vertices.len());
                        for &vid in &poly.vertices {
                            if let Some(v) = input_geo.vertices().get(vid.into()) {
                                pids.push(v.point_id);
                            } else {
                                pids.clear();
                                break;
                            }
                        }
                        if pids.len() < 3 {
                            continue;
                        }
                        let (Some(p0), Some(p1), Some(p2)) = (
                            get_in_pos(pids[0]),
                            get_in_pos(pids[1]),
                            get_in_pos(pids[2]),
                        ) else {
                            continue;
                        };
                        let fnrm = (p1 - p0).cross(p2 - p0).normalize_or_zero();
                        for pid in pids {
                            *n_sum.entry(pid).or_insert(Vec3::ZERO) += fnrm;
                            *n_cnt.entry(pid).or_insert(0.0) += 1.0;
                        }
                    }
                    GeoPrimitive::Polyline(line) => {
                        if let Some(normals) = in_normals {
                            for &vid in &line.vertices {
                                let Some(v) = input_geo.vertices().get(vid.into()) else {
                                    continue;
                                };
                                let Some(di) =
                                    input_geo.points().get_dense_index(v.point_id.into())
                                else {
                                    continue;
                                };
                                if let Some(n) = normals.get(di) {
                                    *n_sum.entry(v.point_id).or_insert(Vec3::ZERO) += *n;
                                    *n_cnt.entry(v.point_id).or_insert(0.0) += 1.0;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            for pid in &selected_points {
                let Some(pos) = get_in_pos(*pid) else {
                    return Arc::new(Geometry::new());
                };
                let cnt = n_cnt.get(pid).copied().unwrap_or(0.0);
                let n = if cnt > 0.0 {
                    (n_sum.get(pid).copied().unwrap_or(Vec3::ZERO) / cnt).normalize_or_zero()
                } else {
                    Vec3::ZERO
                };
                if n.length_squared() <= 1e-20 {
                    return Arc::new(Geometry::new());
                }
                let np = alloc_point(pos + n * distance, &mut out_geo, &mut out_positions);
                front_map.insert(*pid, np);
            }

            // copy unselected primitives
            for (prim_idx, prim) in input_geo.primitives().iter().enumerate() {
                if selection.get(prim_idx) {
                    continue;
                }
                match prim {
                    GeoPrimitive::Polygon(poly) => {
                        let mut verts = Vec::with_capacity(poly.vertices.len());
                        for &vid in &poly.vertices {
                            let Some(v) = input_geo.vertices().get(vid.into()) else {
                                verts.clear();
                                break;
                            };
                            let Some(pn) = get_or_create_base(
                                v.point_id,
                                &mut out_geo,
                                &mut out_positions,
                                &mut base_map,
                            ) else {
                                verts.clear();
                                break;
                            };
                            verts.push(VertexId::from(
                                out_geo
                                    .vertices_mut()
                                    .insert(crate::mesh::GeoVertex { point_id: pn }),
                            ));
                        }
                        if !verts.is_empty() {
                            let _ = out_geo
                                .primitives_mut()
                                .insert(GeoPrimitive::Polygon(PolygonPrim { vertices: verts }));
                            grp_front.push(false);
                            grp_back.push(false);
                            grp_side.push(false);
                        }
                    }
                    GeoPrimitive::Polyline(line) => {
                        let mut verts = Vec::with_capacity(line.vertices.len());
                        for &vid in &line.vertices {
                            let Some(v) = input_geo.vertices().get(vid.into()) else {
                                verts.clear();
                                break;
                            };
                            let Some(pn) = get_or_create_base(
                                v.point_id,
                                &mut out_geo,
                                &mut out_positions,
                                &mut base_map,
                            ) else {
                                verts.clear();
                                break;
                            };
                            verts.push(VertexId::from(
                                out_geo
                                    .vertices_mut()
                                    .insert(crate::mesh::GeoVertex { point_id: pn }),
                            ));
                        }
                        if !verts.is_empty() {
                            let _ = out_geo.primitives_mut().insert(GeoPrimitive::Polyline(
                                PolylinePrim {
                                    vertices: verts,
                                    closed: line.closed,
                                },
                            ));
                            grp_front.push(false);
                            grp_back.push(false);
                            grp_side.push(false);
                        }
                    }
                    _ => {}
                }
            }

            // front/back faces for selected polygons
            for (prim_idx, prim) in input_geo.primitives().iter().enumerate() {
                if !selection.get(prim_idx) {
                    continue;
                }
                let GeoPrimitive::Polygon(poly) = prim else {
                    continue;
                };

                if out_front {
                    let mut verts = Vec::with_capacity(poly.vertices.len());
                    for &vid in &poly.vertices {
                        let Some(v) = input_geo.vertices().get(vid.into()) else {
                            verts.clear();
                            break;
                        };
                        let Some(&pf) = front_map.get(&v.point_id) else {
                            verts.clear();
                            break;
                        };
                        verts.push(VertexId::from(
                            out_geo
                                .vertices_mut()
                                .insert(crate::mesh::GeoVertex { point_id: pf }),
                        ));
                    }
                    if verts.len() >= 3 {
                        let _ = out_geo
                            .primitives_mut()
                            .insert(GeoPrimitive::Polygon(PolygonPrim { vertices: verts }));
                        grp_front.push(true);
                        grp_back.push(false);
                        grp_side.push(false);
                    }
                }

                if out_back {
                    let mut verts = Vec::with_capacity(poly.vertices.len());
                    for &vid in poly.vertices.iter().rev() {
                        let Some(v) = input_geo.vertices().get(vid.into()) else {
                            verts.clear();
                            break;
                        };
                        let Some(pb) = get_or_create_base(
                            v.point_id,
                            &mut out_geo,
                            &mut out_positions,
                            &mut base_map,
                        ) else {
                            verts.clear();
                            break;
                        };
                        verts.push(VertexId::from(
                            out_geo
                                .vertices_mut()
                                .insert(crate::mesh::GeoVertex { point_id: pb }),
                        ));
                    }
                    if verts.len() >= 3 {
                        let _ = out_geo
                            .primitives_mut()
                            .insert(GeoPrimitive::Polygon(PolygonPrim { vertices: verts }));
                        grp_front.push(false);
                        grp_back.push(true);
                        grp_side.push(false);
                    }
                }
            }

            if out_side {
                // polygon boundaries via topology
                for (he_idx, _) in topo.half_edges.iter_enumerated() {
                    let he_id = HalfEdgeId::from(he_idx);
                    let Some(he) = topo.half_edges.get(he_idx) else {
                        continue;
                    };
                    let Some(prim_di) = input_geo
                        .primitives()
                        .get_dense_index(he.primitive_index.into())
                    else {
                        continue;
                    };
                    if !selection.get(prim_di) {
                        continue;
                    }

                    let pair = topo.pair(he_id);
                    let is_boundary = if !pair.is_valid() {
                        true
                    } else {
                        topo.half_edges
                            .get(pair.into())
                            .and_then(|phe| {
                                input_geo
                                    .primitives()
                                    .get_dense_index(phe.primitive_index.into())
                            })
                            .map(|di| !selection.get(di))
                            .unwrap_or(true)
                    };
                    if !is_boundary {
                        continue;
                    }

                    let p1 = he.origin_point;
                    let p2 = topo.dest_point(he_id);
                    let Some(pb1) =
                        get_or_create_base(p1, &mut out_geo, &mut out_positions, &mut base_map)
                    else {
                        continue;
                    };
                    let Some(pb2) =
                        get_or_create_base(p2, &mut out_geo, &mut out_positions, &mut base_map)
                    else {
                        continue;
                    };
                    let pf1 = *front_map.get(&p1).unwrap_or(&pb1);
                    let pf2 = *front_map.get(&p2).unwrap_or(&pb2);

                    let mut last1 = pb1;
                    let mut last2 = pb2;
                    for d in 1..=divisions {
                        let t = d as f32 / divisions as f32;
                        let p1b =
                            out_positions[out_geo.points().get_dense_index(pb1.into()).unwrap()];
                        let p1f =
                            out_positions[out_geo.points().get_dense_index(pf1.into()).unwrap()];
                        let p2b =
                            out_positions[out_geo.points().get_dense_index(pb2.into()).unwrap()];
                        let p2f =
                            out_positions[out_geo.points().get_dense_index(pf2.into()).unwrap()];
                        let c1 = if d == divisions {
                            pf1
                        } else {
                            alloc_point(p1b.lerp(p1f, t), &mut out_geo, &mut out_positions)
                        };
                        let c2 = if d == divisions {
                            pf2
                        } else {
                            alloc_point(p2b.lerp(p2f, t), &mut out_geo, &mut out_positions)
                        };
                        let quad = vec![
                            VertexId::from(
                                out_geo
                                    .vertices_mut()
                                    .insert(crate::mesh::GeoVertex { point_id: last1 }),
                            ),
                            VertexId::from(
                                out_geo
                                    .vertices_mut()
                                    .insert(crate::mesh::GeoVertex { point_id: last2 }),
                            ),
                            VertexId::from(
                                out_geo
                                    .vertices_mut()
                                    .insert(crate::mesh::GeoVertex { point_id: c2 }),
                            ),
                            VertexId::from(
                                out_geo
                                    .vertices_mut()
                                    .insert(crate::mesh::GeoVertex { point_id: c1 }),
                            ),
                        ];
                        let _ = out_geo
                            .primitives_mut()
                            .insert(GeoPrimitive::Polygon(PolygonPrim { vertices: quad }));
                        grp_front.push(false);
                        grp_back.push(false);
                        grp_side.push(true);
                        last1 = c1;
                        last2 = c2;
                    }
                }

                // polyline segments
                for (prim_idx, prim) in input_geo.primitives().iter().enumerate() {
                    if !selection.get(prim_idx) {
                        continue;
                    }
                    let GeoPrimitive::Polyline(line) = prim else {
                        continue;
                    };
                    if line.vertices.len() < 2 {
                        continue;
                    }
                    for i in 0..line.vertices.len() - 1 {
                        let (Some(v1), Some(v2)) = (
                            input_geo.vertices().get(line.vertices[i].into()),
                            input_geo.vertices().get(line.vertices[i + 1].into()),
                        ) else {
                            continue;
                        };
                        let (p1, p2) = (v1.point_id, v2.point_id);
                        let Some(pb1) =
                            get_or_create_base(p1, &mut out_geo, &mut out_positions, &mut base_map)
                        else {
                            continue;
                        };
                        let Some(pb2) =
                            get_or_create_base(p2, &mut out_geo, &mut out_positions, &mut base_map)
                        else {
                            continue;
                        };
                        let pf1 = *front_map.get(&p1).unwrap_or(&pb1);
                        let pf2 = *front_map.get(&p2).unwrap_or(&pb2);

                        let mut last1 = pb1;
                        let mut last2 = pb2;
                        for d in 1..=divisions {
                            let t = d as f32 / divisions as f32;
                            let p1b = out_positions
                                [out_geo.points().get_dense_index(pb1.into()).unwrap()];
                            let p1f = out_positions
                                [out_geo.points().get_dense_index(pf1.into()).unwrap()];
                            let p2b = out_positions
                                [out_geo.points().get_dense_index(pb2.into()).unwrap()];
                            let p2f = out_positions
                                [out_geo.points().get_dense_index(pf2.into()).unwrap()];
                            let c1 = if d == divisions {
                                pf1
                            } else {
                                alloc_point(p1b.lerp(p1f, t), &mut out_geo, &mut out_positions)
                            };
                            let c2 = if d == divisions {
                                pf2
                            } else {
                                alloc_point(p2b.lerp(p2f, t), &mut out_geo, &mut out_positions)
                            };
                            let quad = vec![
                                VertexId::from(
                                    out_geo
                                        .vertices_mut()
                                        .insert(crate::mesh::GeoVertex { point_id: last1 }),
                                ),
                                VertexId::from(
                                    out_geo
                                        .vertices_mut()
                                        .insert(crate::mesh::GeoVertex { point_id: last2 }),
                                ),
                                VertexId::from(
                                    out_geo
                                        .vertices_mut()
                                        .insert(crate::mesh::GeoVertex { point_id: c2 }),
                                ),
                                VertexId::from(
                                    out_geo
                                        .vertices_mut()
                                        .insert(crate::mesh::GeoVertex { point_id: c1 }),
                                ),
                            ];
                            let _ = out_geo
                                .primitives_mut()
                                .insert(GeoPrimitive::Polygon(PolygonPrim { vertices: quad }));
                            grp_front.push(false);
                            grp_back.push(false);
                            grp_side.push(true);
                            last1 = c1;
                            last2 = c2;
                        }
                    }
                }
            }
        }

        out_geo.insert_point_attribute(attrs::P, Attribute::new(out_positions));

        if !front_grp_name.is_empty() {
            out_geo
                .primitive_groups
                .insert(AttributeId::from(front_grp_name.as_str()), grp_front);
        }
        if !back_grp_name.is_empty() {
            out_geo
                .primitive_groups
                .insert(AttributeId::from(back_grp_name.as_str()), grp_back);
        }
        if !side_grp_name.is_empty() {
            out_geo
                .primitive_groups
                .insert(AttributeId::from(side_grp_name.as_str()), grp_side);
        }

        out_geo.calculate_flat_normals();
        Arc::new(out_geo)
    }
}

fn curves_to_polygons_minimal(src: &Geometry, policy: ResamplePolicy) -> Geometry {
    // Build a minimal temp geometry for PolyExtrude: keep polygons/polylines, convert closed curves -> polygons.
    // Preserve primitive groups by index mapping so "Group" parameter continues to work.
    let has_curve = src
        .primitives()
        .iter()
        .any(|p| matches!(p, GeoPrimitive::BezierCurve(_)));
    let Some(p_attr) = src.get_point_attribute(attrs::P) else {
        return Geometry::new();
    };
    let p_slice = p_attr.as_slice::<Vec3>();
    let p_paged = p_attr.as_paged::<Vec3>();
    if p_slice.is_none() && p_paged.is_none() {
        return Geometry::new();
    }
    let (tin_s, tin_p) = if has_curve {
        let Some(a) = src.get_point_attribute(attrs::KNOT_TIN) else {
            return Geometry::new();
        };
        (a.as_slice::<Vec3>(), a.as_paged::<Vec3>())
    } else {
        (None, None)
    };
    let (tout_s, tout_p) = if has_curve {
        let Some(a) = src.get_point_attribute(attrs::KNOT_TOUT) else {
            return Geometry::new();
        };
        (a.as_slice::<Vec3>(), a.as_paged::<Vec3>())
    } else {
        (None, None)
    };
    let (rot_s, rot_p) = if has_curve {
        let Some(a) = src.get_point_attribute(attrs::KNOT_ROT) else {
            return Geometry::new();
        };
        (a.as_slice::<Quat>(), a.as_paged::<Quat>())
    } else {
        (None, None)
    };

    let mut out = Geometry::new();
    let mut out_p: Vec<Vec3> = Vec::new();
    let mut out_n: Vec<Vec3> = Vec::new();
    let mut pid_map: HashMap<PointId, PointId> = HashMap::new(); // source PointId -> out PointId (only for kept prims)
    let mut prim_map: Vec<Option<usize>> = vec![None; src.primitives().len()]; // source prim dense index -> out prim dense index
    let mut source_curve_prim: Vec<i32> = Vec::new(); // per output primitive: source curve prim idx (-1 = not from curve)
    let mut curve_map: Vec<u8> = Vec::new(); // detail bytes mapping (no point-domain fill)
    if has_curve {
        curve_map.extend_from_slice(b"C3DMAP01");
        source_curve_prim.reserve(src.primitives().len());
    }
    let get_p = |di: usize| -> Option<Vec3> {
        p_slice
            .and_then(|s| s.get(di).copied())
            .or_else(|| p_paged.and_then(|pb| pb.get(di)))
    };
    let n_attr = src.get_point_attribute(attrs::N);
    let n_s = n_attr.and_then(|a| a.as_slice::<Vec3>());
    let n_p = n_attr.and_then(|a| a.as_paged::<Vec3>());
    let get_n = |di: usize| -> Vec3 {
        n_s.and_then(|s| s.get(di).copied())
            .or_else(|| n_p.and_then(|pb| pb.get(di)))
            .unwrap_or(Vec3::ZERO)
    };
    let get_tin = |di: usize| -> Option<Vec3> {
        tin_s
            .and_then(|s| s.get(di).copied())
            .or_else(|| tin_p.and_then(|pb| pb.get(di)))
    };
    let get_tout = |di: usize| -> Option<Vec3> {
        tout_s
            .and_then(|s| s.get(di).copied())
            .or_else(|| tout_p.and_then(|pb| pb.get(di)))
    };
    let get_rot = |di: usize| -> Option<Quat> {
        rot_s
            .and_then(|s| s.get(di).copied())
            .or_else(|| rot_p.and_then(|pb| pb.get(di)))
    };
    let pcount = src.points().len();
    if has_curve {
        let ok = (tin_s.map(|s| s.len()).or_else(|| tin_p.map(|pb| pb.len())) == Some(pcount))
            && (tout_s
                .map(|s| s.len())
                .or_else(|| tout_p.map(|pb| pb.len()))
                == Some(pcount))
            && (rot_s.map(|s| s.len()).or_else(|| rot_p.map(|pb| pb.len())) == Some(pcount));
        if !ok {
            return Geometry::new();
        }
    }

    let mut ensure_point = |spid: PointId,
                            out: &mut Geometry,
                            out_p: &mut Vec<Vec3>,
                            out_n: &mut Vec<Vec3>,
                            pid_map: &mut HashMap<PointId, PointId>|
     -> Option<PointId> {
        if let Some(&p) = pid_map.get(&spid) {
            return Some(p);
        }
        let di = src.points().get_dense_index(spid.into())?;
        let pid = PointId::from(out.points_mut().insert(()));
        out_p.push(get_p(di)?);
        out_n.push(get_n(di));
        pid_map.insert(spid, pid);
        Some(pid)
    };

    // 1) Copy polygons + polylines as-is (topology-friendly) using shared points.
    // Fill curve-source prim attr for these prims as -1 (explicit non-curve provenance).
    for (prim_i, prim) in src.primitives().iter().enumerate() {
        match prim {
            GeoPrimitive::Polygon(poly) => {
                let mut vs: Vec<VertexId> = Vec::with_capacity(poly.vertices.len());
                for &vid in &poly.vertices {
                    let Some(v) = src.vertices().get(vid.into()) else {
                        vs.clear();
                        break;
                    };
                    let Some(op) =
                        ensure_point(v.point_id, &mut out, &mut out_p, &mut out_n, &mut pid_map)
                    else {
                        vs.clear();
                        break;
                    };
                    vs.push(VertexId::from(
                        out.vertices_mut()
                            .insert(crate::mesh::GeoVertex { point_id: op }),
                    ));
                }
                if vs.len() >= 3 {
                    let idx = out
                        .primitives_mut()
                        .insert(GeoPrimitive::Polygon(PolygonPrim { vertices: vs }));
                    let dense = out.primitives().get_dense_index(idx).unwrap();
                    prim_map[prim_i] = Some(dense);
                    if has_curve {
                        source_curve_prim.push(-1);
                    }
                }
            }
            GeoPrimitive::Polyline(line) => {
                let mut vs: Vec<VertexId> = Vec::with_capacity(line.vertices.len());
                for &vid in &line.vertices {
                    let Some(v) = src.vertices().get(vid.into()) else {
                        vs.clear();
                        break;
                    };
                    let Some(op) =
                        ensure_point(v.point_id, &mut out, &mut out_p, &mut out_n, &mut pid_map)
                    else {
                        vs.clear();
                        break;
                    };
                    vs.push(VertexId::from(
                        out.vertices_mut()
                            .insert(crate::mesh::GeoVertex { point_id: op }),
                    ));
                }
                if vs.len() >= 2 {
                    let idx = out
                        .primitives_mut()
                        .insert(GeoPrimitive::Polyline(PolylinePrim {
                            vertices: vs,
                            closed: line.closed,
                        }));
                    let dense = out.primitives().get_dense_index(idx).unwrap();
                    prim_map[prim_i] = Some(dense);
                    if has_curve {
                        source_curve_prim.push(-1);
                    }
                }
            }
            _ => {}
        }
    }

    // 2) Convert curves -> polygons (closed) or polylines (open).
    for (prim_i, prim) in src.primitives().iter().enumerate() {
        let GeoPrimitive::BezierCurve(c) = prim else {
            continue;
        };
        if c.vertices.len() < 2 {
            continue;
        }
        let mut knots: Vec<
            crate::libs::algorithms::algorithms_runtime::unity_spline::unity_spline::BezierKnot,
        > = Vec::with_capacity(c.vertices.len());
        for &vid in &c.vertices {
            let Some(v) = src.vertices().get(vid.into()) else {
                knots.clear();
                break;
            };
            let Some(di) = src.points().get_dense_index(v.point_id.into()) else {
                knots.clear();
                break;
            };
            let Some(position) = get_p(di) else {
                return Geometry::new();
            };
            let Some(tangent_in) = get_tin(di) else {
                return Geometry::new();
            };
            let Some(tangent_out) = get_tout(di) else {
                return Geometry::new();
            };
            let Some(rotation) = get_rot(di) else {
                return Geometry::new();
            };
            knots.push(crate::libs::algorithms::algorithms_runtime::unity_spline::unity_spline::BezierKnot { position, tangent_in, tangent_out, rotation });
        }
        if c.closed && knots.len() < 3 {
            continue;
        }
        let spans = if c.closed {
            knots.len()
        } else {
            knots.len() - 1
        };
        let (mut rp, mut u, mut sp) = resample_bezier_knots(&knots, c.closed, policy);
        if rp.len() > 3 {
            let a = rp[0];
            if let Some(b) = rp.last().copied() {
                if (a - b).length_squared() < 1e-10 {
                    rp.pop();
                    u.pop();
                    sp.pop();
                }
            }
        }
        if rp.len() < 2 || rp.len() != u.len() || rp.len() != sp.len() {
            return Geometry::new();
        }
        let mut vs: Vec<VertexId> = Vec::with_capacity(rp.len());
        let start = out_p.len() as u32;
        let count = rp.len() as u32;
        for j in 0..count as usize {
            let p = rp[j];
            let pid = PointId::from(out.points_mut().insert(()));
            out_p.push(p);
            let span = sp[j];
            if span >= spans {
                return Geometry::new();
            }
            let a = knots[span].rotation;
            let b = knots[(span + 1) % knots.len()].rotation;
            let alpha = ((u[j] * spans as f32) - span as f32).clamp(0.0, 1.0);
            let rot = a.slerp(b, alpha);
            out_n.push(rot.mul_vec3(Vec3::Y).normalize_or_zero());
            vs.push(VertexId::from(
                out.vertices_mut()
                    .insert(crate::mesh::GeoVertex { point_id: pid }),
            ));
        }
        let prim_out = if c.closed {
            if vs.len() < 3 {
                continue;
            }
            GeoPrimitive::Polygon(PolygonPrim { vertices: vs })
        } else {
            GeoPrimitive::Polyline(PolylinePrim {
                vertices: vs,
                closed: false,
            })
        };
        let idx = out.primitives_mut().insert(prim_out);
        let dense = out.primitives().get_dense_index(idx).unwrap();
        prim_map[prim_i] = Some(dense);
        source_curve_prim.push(prim_i as i32);
        if has_curve {
            curve_map.extend_from_slice(&(dense as u32).to_le_bytes());
            curve_map.extend_from_slice(&(prim_i as i32).to_le_bytes());
            curve_map.extend_from_slice(&start.to_le_bytes());
            curve_map.extend_from_slice(&count.to_le_bytes());
            for j in 0..count as usize {
                curve_map.extend_from_slice(&u[j].to_le_bytes());
                curve_map.extend_from_slice(&(sp[j] as u32).to_le_bytes());
            }
        }
    }

    out.insert_point_attribute(attrs::P, Attribute::new_auto(out_p));
    out.insert_point_attribute(attrs::N, Attribute::new_auto(out_n));
    if has_curve && source_curve_prim.len() == out.primitives().len() {
        out.insert_primitive_attribute(
            AttributeId::from("__cunning.source_curve_prim"),
            Attribute::new_auto(source_curve_prim),
        );
    }
    if has_curve && curve_map.len() > 8 {
        out.set_detail_attribute(
            "__cunning.curve_to_polygon_map",
            crate::libs::geometry::mesh::Bytes(curve_map),
        );
    }

    // 3) Remap primitive groups (mask indices follow dense primitive order)
    for (name, mask) in &src.primitive_groups {
        let mut out_mask = ElementGroupMask::new(out.primitives().len());
        for i in mask.iter_ones() {
            if let Some(Some(ni)) = prim_map.get(i).map(|x| *x) {
                out_mask.set(ni, true);
            }
        }
        out.primitive_groups.insert(*name, out_mask);
    }

    out
}

register_node!("PolyExtrude", "Modeling", PolyExtrudeNode);

// Helpers
fn get_param_string(params: &[Parameter], name: &str, default: &str) -> String {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or(default.to_string())
}

fn get_param_int(params: &[Parameter], name: &str, default: i32) -> i32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::Int(i) => Some(*i),
            _ => None,
        })
        .unwrap_or(default)
}

fn get_param_float(params: &[Parameter], name: &str, default: f32) -> f32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::Float(f) => Some(*f),
            _ => None,
        })
        .unwrap_or(default)
}

fn get_param_bool(params: &[Parameter], name: &str, default: bool) -> bool {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::Bool(b) => Some(*b),
            _ => None,
        })
        .unwrap_or(default)
}
