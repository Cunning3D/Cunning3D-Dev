//! OBJ importer (using the tobj crate)
use super::{FileImporter, FileMetadata};
use crate::libs::geometry::attrs;
use crate::libs::geometry::mesh::{Attribute, GeoPrimitive, Geometry, PolygonPrim};
use bevy::prelude::{Vec2, Vec3};
use std::path::Path;

pub struct ObjImporter;

impl FileImporter for ObjImporter {
    fn extensions(&self) -> &[&str] {
        &["obj"]
    }

    fn import(&self, path: &Path) -> Result<Geometry, String> {
        let (models, _materials) = tobj::load_obj(path, &tobj::GPU_LOAD_OPTIONS)
            .map_err(|e| format!("OBJ load error: {}", e))?;

        let mut geo = Geometry::new();

        for model in models {
            let mesh = &model.mesh;
            let pt_off = geo.points().len();
            let vt_off = geo.vertices().len();

            // Add points
            let positions: Vec<Vec3> = mesh
                .positions
                .chunks(3)
                .map(|c| Vec3::new(c[0], c[1], c[2]))
                .collect();

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

            // Parse faces
            let mut vertex_uvs: Vec<Vec2> = Vec::new();
            for face_start in (0..mesh.indices.len()).step_by(3) {
                let mut poly_vids = Vec::new();
                for j in 0..3 {
                    let idx = mesh.indices[face_start + j] as usize + pt_off;
                    if let Some(pid) = geo.points().get_id_from_dense(idx) {
                        let vid = geo.add_vertex(pid.into());
                        poly_vids.push(vid);

                        // UV
                        if !mesh.texcoords.is_empty() {
                            let ti = mesh
                                .texcoord_indices
                                .get(face_start + j)
                                .copied()
                                .unwrap_or(0) as usize;
                            let u = mesh.texcoords.get(ti * 2).copied().unwrap_or(0.0);
                            let v = mesh.texcoords.get(ti * 2 + 1).copied().unwrap_or(0.0);
                            vertex_uvs.push(Vec2::new(u, v));
                        }
                    }
                }
                if poly_vids.len() == 3 {
                    geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim {
                        vertices: poly_vids,
                    }));
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

        if geo.primitives().len() > 0 {
            geo.calculate_flat_normals();
        }
        Ok(geo)
    }
}
