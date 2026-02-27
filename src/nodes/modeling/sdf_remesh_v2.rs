use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct SdfRemeshV2Node;

impl NodeParameters for SdfRemeshV2Node {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new("voxel_size", "Voxel Size", "SDF", ParameterValue::Float(0.02), ParameterUIType::FloatSlider { min: 0.001, max: 1.0 }),
            Parameter::new("bandwidth", "Bandwidth", "SDF", ParameterValue::Int(3), ParameterUIType::IntSlider { min: 1, max: 12 }),
            Parameter::new("iso_value", "Iso Value", "Extract", ParameterValue::Float(0.0), ParameterUIType::FloatSlider { min: -1.0, max: 1.0 }),
            Parameter::new("hard_surface", "Hard Surface", "Extract", ParameterValue::Bool(false), ParameterUIType::Toggle),
        ]
    }
}

impl NodeOp for SdfRemeshV2Node {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let input = inputs.first().map(|g| g.materialize()).unwrap_or_else(Geometry::new);
        let p: HashMap<String, ParameterValue> = params.iter().map(|p| (p.name.clone(), p.value.clone())).collect();
        Arc::new(compute_sdf_remesh_v2(&input, &p))
    }
}

register_node!("SDF Remesh V2", "Modeling", SdfRemeshV2Node);

pub fn compute_sdf_remesh_v2(input_geo: &Geometry, params: &HashMap<String, ParameterValue>) -> Geometry {
    let voxel_size = match params.get("voxel_size") { Some(ParameterValue::Float(v)) => *v, _ => 0.02 }.max(1e-4);
    let bandwidth = match params.get("bandwidth") { Some(ParameterValue::Int(v)) => *v, _ => 3 }.clamp(1, 64);
    let iso_value = match params.get("iso_value") { Some(ParameterValue::Float(v)) => *v, _ => 0.0 };
    let hard_surface = match params.get("hard_surface") { Some(ParameterValue::Bool(v)) => *v, _ => false };

    // Reuse the existing robust mesh→SDF and SDF→mesh pipeline.
    let mut p_sdf = HashMap::new();
    p_sdf.insert("voxel_size".to_string(), ParameterValue::Float(voxel_size));
    p_sdf.insert("bandwidth".to_string(), ParameterValue::Int(bandwidth));
    p_sdf.insert("display_points".to_string(), ParameterValue::Bool(false));
    let sdf_geo = crate::nodes::sdf::sdf_from_mesh::compute_sdf_from_mesh(input_geo, &p_sdf);
    if sdf_geo.sdfs.is_empty() { return Geometry::new(); }

    let meshes: Vec<Geometry> = sdf_geo
        .sdfs
        .iter()
        .map(|h| crate::nodes::sdf::sdf_to_mesh::sdf_to_geometry(h, iso_value, false, hard_surface))
        .collect();
    let mut out = crate::libs::algorithms::merge::merge_geometry_slice(&meshes);
    out.calculate_smooth_normals();
    out
}
