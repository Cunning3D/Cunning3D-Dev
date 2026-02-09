use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use bevy::prelude::Vec3;
use std::sync::Arc;

#[derive(Default)]
pub struct AttributeKernelGpuNode;

impl NodeParameters for AttributeKernelGpuNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "domain",
                "Domain",
                "GPU",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![("Points".into(), 0)],
                },
            ),
            Parameter::new(
                "attr",
                "Attribute",
                "GPU",
                ParameterValue::String(attrs::P.into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "op",
                "Op",
                "GPU",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![("Add".into(), 0), ("Mul".into(), 1), ("Set".into(), 2)],
                },
            ),
            Parameter::new(
                "value",
                "Value",
                "GPU",
                ParameterValue::Vec3(Vec3::new(0.0, 1.0, 0.0)),
                ParameterUIType::Vec3Drag,
            ),
        ]
    }
}

impl NodeOp for AttributeKernelGpuNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        // Legacy NodeOp path materializes to CPU; runtime uses `compute_output_ref` to keep GPU-resident results.
        let in_geo = Arc::new(
            inputs
                .first()
                .map(|g| g.materialize())
                .unwrap_or_else(Geometry::new),
        );
        let op = crate::nodes::gpu::ops::lower_attribute_kernel(params);
        let crate::nodes::gpu::ops::GpuOp::AffineVec3 {
            domain,
            attr,
            mul,
            add,
        } = op
        else {
            return in_geo;
        };
        if domain != 0 {
            return in_geo;
        }
        let h = crate::nodes::gpu::runtime::GpuGeoHandle::from_cpu(in_geo.clone())
            .apply_affine_vec3(attr.as_str(), mul, add);
        h.download_affine_attr_vec3_blocking(attr.as_str())
    }
}

crate::register_node!(
    "Attribute Kernel (GPU)",
    "Attribute",
    crate::nodes::gpu::attribute_kernel::AttributeKernelGpuNode
);
