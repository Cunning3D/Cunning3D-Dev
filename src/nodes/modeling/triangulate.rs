use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::mesh::{GeoPrimitive, Geometry, PolygonPrim};
use crate::nodes::parameter::Parameter;
use crate::register_node;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct TriangulateNode;

impl NodeParameters for TriangulateNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![]
    }
}

impl NodeOp for TriangulateNode {
    fn compute(&self, _params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let input = match mats.get(0) {
            Some(g) => g,
            None => return Arc::new(Geometry::new()),
        };

        let mut out_geo = input.fork();

        use crate::libs::geometry::ids::PrimId;
        let mut to_remove = Vec::new();
        let mut new_prims = Vec::new();

        for (idx, prim) in out_geo.primitives().iter_enumerated() {
            if let GeoPrimitive::Polygon(poly) = prim {
                if poly.vertices.len() > 3 {
                    to_remove.push(PrimId::from(idx));

                    let v0 = poly.vertices[0];
                    for i in 1..poly.vertices.len() - 1 {
                        let v1 = poly.vertices[i];
                        let v2 = poly.vertices[i + 1];
                        new_prims.push(GeoPrimitive::Polygon(PolygonPrim {
                            vertices: vec![v0, v1, v2],
                        }));
                    }
                }
            }
        }

        for pid in to_remove {
            out_geo.remove_primitive(pid);
        }

        for prim in new_prims {
            out_geo.add_primitive(prim);
        }

        Arc::new(out_geo)
    }
}

register_node!("Triangulate", "Modeling", TriangulateNode);
