use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::mesh::{Attribute, GeoPrimitive, Geometry};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use rayon::prelude::*;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct MeasureNode;

impl NodeParameters for MeasureNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "type",
                "Measure Type",
                "General",
                ParameterValue::String("Area".to_string()),
                ParameterUIType::String, // TODO: Dropdown
            ),
            Parameter::new(
                "attribute",
                "Attribute Name",
                "General",
                ParameterValue::String("".to_string()),
                ParameterUIType::String,
            ),
        ]
    }
}

impl NodeOp for MeasureNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let input = match mats.get(0) {
            Some(g) => g,
            None => return Arc::new(Geometry::new()),
        };

        let measure_type = params
            .iter()
            .find(|p| p.name == "type")
            .and_then(|p| match &p.value {
                ParameterValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or("Area".to_string());

        let mut out_geo = input.fork();
        let positions = match out_geo.get_point_position_attribute() {
            Some(p) => p.to_vec(),
            None => return Arc::new(out_geo),
        };

        // Parallel computation utilizing ECS dense primitive array
        // We capture 'positions' slice and geometry reference
        // Note: get_pos_by_vertex access is thread-safe (read-only)
        let values: Vec<f32> = out_geo
            .primitives()
            .values()
            .par_iter()
            .map(|prim| match prim {
                GeoPrimitive::Polygon(poly) => {
                    if measure_type == "Perimeter" {
                        let mut len = 0.0;
                        if poly.vertices.len() >= 2 {
                            for i in 0..poly.vertices.len() {
                                let p1 = input.get_pos_by_vertex(poly.vertices[i], &positions);
                                let p2 = input.get_pos_by_vertex(
                                    poly.vertices[(i + 1) % poly.vertices.len()],
                                    &positions,
                                );
                                len += (p1 - p2).length();
                            }
                        }
                        len
                    } else {
                        let mut area = 0.0;
                        if poly.vertices.len() >= 3 {
                            let p0 = input.get_pos_by_vertex(poly.vertices[0], &positions);
                            for i in 1..poly.vertices.len() - 1 {
                                let p1 = input.get_pos_by_vertex(poly.vertices[i], &positions);
                                let p2 = input.get_pos_by_vertex(poly.vertices[i + 1], &positions);
                                area += (p1 - p0).cross(p2 - p0).length() * 0.5;
                            }
                        }
                        area
                    }
                }
                GeoPrimitive::Polyline(line) => {
                    let mut len = 0.0;
                    if line.vertices.len() >= 2 {
                        for i in 0..line.vertices.len() - 1 {
                            let p1 = input.get_pos_by_vertex(line.vertices[i], &positions);
                            let p2 = input.get_pos_by_vertex(line.vertices[i + 1], &positions);
                            len += (p1 - p2).length();
                        }
                        if line.closed {
                            let p1 = input.get_pos_by_vertex(
                                line.vertices[line.vertices.len() - 1],
                                &positions,
                            );
                            let p2 = input.get_pos_by_vertex(line.vertices[0], &positions);
                            len += (p1 - p2).length();
                        }
                    }
                    len
                }
                _ => 0.0,
            })
            .collect();

        let default_name = if measure_type == "Perimeter" {
            "perimeter"
        } else {
            "area"
        };
        let attr_name = params
            .iter()
            .find(|p| p.name == "attribute")
            .and_then(|p| match &p.value {
                ParameterValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .filter(|s| !s.is_empty())
            .unwrap_or(default_name.to_string());

        out_geo.insert_primitive_attribute(attr_name, Attribute::new(values));

        Arc::new(out_geo)
    }
}

register_node!("Measure", "Attribute", MeasureNode);
