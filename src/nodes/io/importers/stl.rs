//! STL importer (using the stl_io crate)
use super::{FileImporter, FileMetadata};
use crate::libs::geometry::attrs;
use crate::libs::geometry::mesh::{Attribute, GeoPrimitive, Geometry, PolygonPrim};
use bevy::prelude::Vec3;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::Path;

pub struct StlImporter;

impl FileImporter for StlImporter {
    fn extensions(&self) -> &[&str] {
        &["stl"]
    }

    fn import(&self, path: &Path) -> Result<Geometry, String> {
        let mut file = OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|e| format!("Failed to open STL: {}", e))?;

        let stl = stl_io::read_stl(&mut file).map_err(|e| format!("STL parse error: {}", e))?;

        let mut geo = Geometry::new();

        // STL is a triangle soup; merge duplicate vertices
        let mut vertex_map: HashMap<[i32; 3], usize> = HashMap::new();
        let mut positions: Vec<Vec3> = Vec::new();

        let quantize = |v: f32| (v * 10000.0) as i32;

        for tri in &stl.faces {
            let mut poly_vids = Vec::new();

            // stl_io: IndexedTriangle.vertices are indices into stl.vertices
            for &vert_idx in &tri.vertices {
                let vert = stl.vertices[vert_idx];
                let k = [quantize(vert[0]), quantize(vert[1]), quantize(vert[2])];
                let pt_idx = *vertex_map.entry(k).or_insert_with(|| {
                    let idx = positions.len();
                    positions.push(Vec3::new(vert[0], vert[1], vert[2]));
                    geo.add_point();
                    idx
                });

                if let Some(pid) = geo.points().get_id_from_dense(pt_idx) {
                    poly_vids.push(geo.add_vertex(pid.into()));
                }
            }

            if poly_vids.len() == 3 {
                geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim {
                    vertices: poly_vids,
                }));
            }
        }

        // Set positions
        geo.insert_point_attribute(attrs::P, Attribute::new(positions));

        if geo.primitives().len() > 0 {
            geo.calculate_flat_normals();
        }
        Ok(geo)
    }
}
