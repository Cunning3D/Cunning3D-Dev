//! FBX 导入器
use super::{FileImporter, FileMetadata};
use crate::libs::geometry::attrs;
use crate::libs::geometry::ids::VertexId;
use crate::libs::geometry::mesh::{Attribute, GeoPrimitive, Geometry, PolygonPrim};
use bevy::prelude::{Vec2, Vec3};
use fbxcel::tree::any::AnyTree;
use fbxcel::tree::v7400::{NodeHandle, Tree};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

pub struct FbxImporter;

impl FileImporter for FbxImporter {
    fn extensions(&self) -> &[&str] {
        &["fbx"]
    }

    fn import(&self, path: &Path) -> Result<Geometry, String> {
        let file = File::open(path).map_err(|e| format!("Failed to open: {}", e))?;
        let tree = match AnyTree::from_seekable_reader(BufReader::new(file))
            .map_err(|e| format!("FBX parse error: {}", e))?
        {
            AnyTree::V7400(_, tree, _) => tree,
            _ => return Err("Unsupported FBX version".into()),
        };
        parse_fbx_tree(&tree)
    }
}

struct FbxData {
    positions: Vec<Vec3>,
    indices: Vec<i32>,
    uvs: Vec<Vec2>,
    uv_indices: Vec<i32>,
}

fn parse_fbx_tree(tree: &Tree) -> Result<Geometry, String> {
    let mut geo = Geometry::new();
    let root = tree.root();

    if let Some(objects) = root.children().find(|n| n.name() == "Objects") {
        for obj in objects.children().filter(|n| n.name() == "Geometry") {
            if let Ok(data) = parse_geometry_node(&obj) {
                let pt_off = geo.points().len();
                let vt_off = geo.vertices().len();

                for _ in &data.positions {
                    geo.add_point();
                }

                if geo.get_point_attribute(attrs::P).is_none() {
                    geo.insert_point_attribute(
                        attrs::P,
                        Attribute::new(vec![Vec3::ZERO; geo.points().len()]),
                    );
                }
                if let Some(attr) = geo.get_point_attribute_mut(attrs::P) {
                    if let Some(s) = attr.as_mut_slice::<Vec3>() {
                        for (i, p) in data.positions.iter().enumerate() {
                            if let Some(slot) = s.get_mut(pt_off + i) {
                                *slot = *p;
                            }
                        }
                    }
                }

                let mut vuv: Vec<Vec2> = Vec::new();
                let mut poly: Vec<VertexId> = Vec::new();
                let mut vi = 0usize;

                for &idx in &data.indices {
                    let ai = if idx < 0 { -(idx + 1) } else { idx } as usize + pt_off;
                    if ai < geo.points().len() {
                        if let Some(pid) = geo.points().get_id_from_dense(ai) {
                            poly.push(geo.add_vertex(pid.into()));
                            vuv.push(get_uv(&data, vi));
                            vi += 1;
                        }
                    }
                    if idx < 0 && poly.len() >= 3 {
                        geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim {
                            vertices: poly.clone(),
                        }));
                        poly.clear();
                    }
                }

                if !vuv.is_empty() {
                    if geo.get_vertex_attribute(attrs::UV).is_none() {
                        geo.insert_vertex_attribute(
                            attrs::UV,
                            Attribute::new(vec![Vec2::ZERO; geo.vertices().len()]),
                        );
                    }
                    if let Some(attr) = geo.get_vertex_attribute_mut(attrs::UV) {
                        if let Some(s) = attr.as_mut_slice::<Vec2>() {
                            for (i, uv) in vuv.iter().enumerate() {
                                if let Some(slot) = s.get_mut(vt_off + i) {
                                    *slot = *uv;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if geo.primitives().len() > 0 {
        geo.calculate_flat_normals();
    }
    Ok(geo)
}

fn get_uv(d: &FbxData, vi: usize) -> Vec2 {
    if d.uvs.is_empty() {
        return Vec2::ZERO;
    }
    let i = if !d.uv_indices.is_empty() {
        d.uv_indices.get(vi).copied().unwrap_or(0) as usize
    } else {
        vi
    };
    d.uvs.get(i).copied().unwrap_or(Vec2::ZERO)
}

fn parse_geometry_node(node: &NodeHandle) -> Result<FbxData, String> {
    let mut d = FbxData {
        positions: Vec::new(),
        indices: Vec::new(),
        uvs: Vec::new(),
        uv_indices: Vec::new(),
    };
    for c in node.children() {
        match c.name() {
            "Vertices" => {
                if let Some(a) = c.attributes().get(0) {
                    if let Some(arr) = a.get_arr_f64() {
                        for ch in arr.chunks(3) {
                            if ch.len() == 3 {
                                d.positions.push(Vec3::new(
                                    ch[0] as f32,
                                    ch[1] as f32,
                                    ch[2] as f32,
                                ));
                            }
                        }
                    }
                }
            }
            "PolygonVertexIndex" => {
                if let Some(a) = c.attributes().get(0) {
                    if let Some(arr) = a.get_arr_i32() {
                        d.indices = arr.to_vec();
                    }
                }
            }
            "LayerElementUV" => {
                for s in c.children() {
                    match s.name() {
                        "UV" => {
                            if let Some(a) = s.attributes().get(0) {
                                if let Some(arr) = a.get_arr_f64() {
                                    for ch in arr.chunks(2) {
                                        if ch.len() == 2 {
                                            d.uvs.push(Vec2::new(ch[0] as f32, ch[1] as f32));
                                        }
                                    }
                                }
                            }
                        }
                        "UVIndex" => {
                            if let Some(a) = s.attributes().get(0) {
                                if let Some(arr) = a.get_arr_i32() {
                                    d.uv_indices = arr.to_vec();
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if d.positions.is_empty() {
        return Err("No vertices".into());
    }
    if d.indices.is_empty() {
        return Err("No indices".into());
    }
    Ok(d)
}
