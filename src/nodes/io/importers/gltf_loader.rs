//! glTF importer (using the gltf crate)
use super::{FileImporter, FileMetadata};
use crate::libs::geometry::attrs;
use crate::libs::geometry::mesh::{Attribute, GeoPrimitive, Geometry, PolygonPrim};
use bevy::prelude::{Vec2, Vec3};
use std::path::Path;

pub struct GltfImporter;

impl FileImporter for GltfImporter {
    fn extensions(&self) -> &[&str] {
        &["gltf", "glb"]
    }

    fn import(&self, path: &Path) -> Result<Geometry, String> {
        let (document, buffers, _images) =
            gltf::import(path).map_err(|e| format!("glTF load error: {}", e))?;

        let mut geo = Geometry::new();

        for mesh in document.meshes() {
            for primitive in mesh.primitives() {
                let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

                let pt_off = geo.points().len();
                let vt_off = geo.vertices().len();

                // Read positions
                let positions: Vec<Vec3> = reader
                    .read_positions()
                    .map(|iter| iter.map(|p| Vec3::new(p[0], p[1], p[2])).collect())
                    .unwrap_or_default();

                if positions.is_empty() {
                    continue;
                }

                for _ in &positions {
                    geo.add_point();
                }

                // Set positions
                if geo.get_point_attribute(attrs::P).is_none() {
                    geo.insert_point_attribute(
                        attrs::P,
                        Attribute::new(vec![Vec3::ZERO; geo.points().len()]),
                    );
                }
                if let Some(attr) = geo.get_point_attribute_mut(attrs::P) {
                    if let Some(s) = attr.as_mut_slice::<Vec3>() {
                        for (i, p) in positions.iter().enumerate() {
                            if let Some(slot) = s.get_mut(pt_off + i) {
                                *slot = *p;
                            }
                        }
                    }
                }

                // Read UVs
                let uvs: Vec<Vec2> = reader
                    .read_tex_coords(0)
                    .map(|tc| tc.into_f32().map(|uv| Vec2::new(uv[0], uv[1])).collect())
                    .unwrap_or_default();

                // Read indices and create faces
                let mut vertex_uvs: Vec<Vec2> = Vec::new();
                if let Some(indices) = reader.read_indices() {
                    let indices: Vec<u32> = indices.into_u32().collect();
                    for tri in indices.chunks(3) {
                        if tri.len() != 3 {
                            continue;
                        }
                        let mut poly_vids = Vec::new();
                        for &idx in tri {
                            let pi = idx as usize + pt_off;
                            if let Some(pid) = geo.points().get_id_from_dense(pi) {
                                let vid = geo.add_vertex(pid.into());
                                poly_vids.push(vid);
                                vertex_uvs
                                    .push(uvs.get(idx as usize).copied().unwrap_or(Vec2::ZERO));
                            }
                        }
                        if poly_vids.len() == 3 {
                            geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim {
                                vertices: poly_vids,
                            }));
                        }
                    }
                }

                // Set UVs
                if !vertex_uvs.is_empty() {
                    if geo.get_vertex_attribute(attrs::UV).is_none() {
                        geo.insert_vertex_attribute(
                            attrs::UV,
                            Attribute::new(vec![Vec2::ZERO; geo.vertices().len()]),
                        );
                    }
                    if let Some(attr) = geo.get_vertex_attribute_mut(attrs::UV) {
                        if let Some(s) = attr.as_mut_slice::<Vec2>() {
                            for (i, uv) in vertex_uvs.iter().enumerate() {
                                if let Some(slot) = s.get_mut(vt_off + i) {
                                    *slot = *uv;
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
}
