//! PLY importer (using the ply-rs crate)
use super::{FileImporter, FileMetadata};
use crate::libs::geometry::attrs;
use crate::libs::geometry::mesh::{Attribute, GeoPrimitive, Geometry, PolygonPrim};
use bevy::prelude::Vec3;
use ply_rs::parser::Parser;
use ply_rs::ply::{ElementDef, PropertyAccess, PropertyDef, PropertyType, ScalarType};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

pub struct PlyImporter;

impl FileImporter for PlyImporter {
    fn extensions(&self) -> &[&str] {
        &["ply"]
    }

    fn import(&self, path: &Path) -> Result<Geometry, String> {
        let file = File::open(path).map_err(|e| format!("Failed to open PLY: {}", e))?;
        let mut reader = BufReader::new(file);

        let vertex_parser = Parser::<Vertex>::new();
        let face_parser = Parser::<Face>::new();

        let header = vertex_parser
            .read_header(&mut reader)
            .map_err(|e| format!("PLY header error: {}", e))?;

        let mut geo = Geometry::new();
        let mut positions: Vec<Vec3> = Vec::new();
        let mut faces: Vec<Vec<usize>> = Vec::new();

        for (name, element) in &header.elements {
            match name.as_str() {
                "vertex" => {
                    let verts = vertex_parser
                        .read_payload_for_element(&mut reader, element, &header)
                        .map_err(|e| format!("PLY vertex error: {}", e))?;
                    for v in verts {
                        positions.push(Vec3::new(v.x, v.y, v.z));
                        geo.add_point();
                    }
                }
                "face" => {
                    let fs = face_parser
                        .read_payload_for_element(&mut reader, element, &header)
                        .map_err(|e| format!("PLY face error: {}", e))?;
                    for f in fs {
                        if f.indices.len() >= 3 {
                            faces.push(f.indices.iter().map(|&i| i as usize).collect());
                        }
                    }
                }
                _ => {}
            }
        }

        // Set positions
        geo.insert_point_attribute(attrs::P, Attribute::new(positions));

        // Create faces
        for face_indices in faces {
            let mut poly_vids = Vec::new();
            for idx in face_indices {
                if let Some(pid) = geo.points().get_id_from_dense(idx) {
                    poly_vids.push(geo.add_vertex(pid.into()));
                }
            }
            if poly_vids.len() >= 3 {
                geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim {
                    vertices: poly_vids,
                }));
            }
        }

        if geo.primitives().len() > 0 {
            geo.calculate_flat_normals();
        }
        Ok(geo)
    }
}

#[derive(Debug, Default)]
struct Vertex {
    x: f32,
    y: f32,
    z: f32,
}

impl PropertyAccess for Vertex {
    fn new() -> Self {
        Self::default()
    }
    fn set_property(&mut self, key: String, property: ply_rs::ply::Property) {
        match (key.as_str(), property) {
            ("x", ply_rs::ply::Property::Float(v)) => self.x = v,
            ("y", ply_rs::ply::Property::Float(v)) => self.y = v,
            ("z", ply_rs::ply::Property::Float(v)) => self.z = v,
            ("x", ply_rs::ply::Property::Double(v)) => self.x = v as f32,
            ("y", ply_rs::ply::Property::Double(v)) => self.y = v as f32,
            ("z", ply_rs::ply::Property::Double(v)) => self.z = v as f32,
            _ => {}
        }
    }
}

#[derive(Debug, Default)]
struct Face {
    indices: Vec<u32>,
}

impl PropertyAccess for Face {
    fn new() -> Self {
        Self::default()
    }
    fn set_property(&mut self, key: String, property: ply_rs::ply::Property) {
        if key == "vertex_indices" || key == "vertex_index" {
            if let ply_rs::ply::Property::ListUInt(v) = property {
                self.indices = v;
            } else if let ply_rs::ply::Property::ListInt(v) = property {
                self.indices = v.iter().map(|&i| i as u32).collect();
            }
        }
    }
}
