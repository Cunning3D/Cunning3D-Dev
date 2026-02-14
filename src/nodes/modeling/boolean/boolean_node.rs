use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::algorithms::boolean::{
    manifold_backend::{
        run_manifold_boolean, ManifoldBooleanSettings, NormalStrategy, OutputTopology,
    },
    BooleanOperation,
};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct BooleanNode;

impl NodeParameters for BooleanNode {
    // Define the parameters for the UI, matching Houdini's layout
    fn define_parameters() -> Vec<Parameter> {
        vec![
            // --- Inputs ---
            Parameter::new(
                "group_a",
                "Group A",
                "Inputs",
                ParameterValue::String("".to_string()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "group_b",
                "Group B",
                "Inputs",
                ParameterValue::String("".to_string()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "input_type",
                "Treat As",
                "Inputs",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![("Solid".to_string(), 0), ("Surface".to_string(), 1)],
                },
            ),
            // --- Operation ---
            Parameter::new(
                "operation",
                "Operation",
                "Settings",
                ParameterValue::Int(2), // Default Subtract
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Union".to_string(), 0),
                        ("Intersect".to_string(), 1),
                        ("Subtract".to_string(), 2),
                        ("Difference".to_string(), 3),
                    ],
                },
            ),
            Parameter::new(
                "subtract_mode",
                "Subtract",
                "Settings",
                ParameterValue::Int(0), // Default A - B
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("A - B".to_string(), 0),
                        ("B - A".to_string(), 1),
                        ("Both".to_string(), 2),
                    ],
                },
            ),
            // --- Output Geometry Settings ---
            Parameter::new(
                "output_topology",
                "Output Topology",
                "Settings",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Polygons (N-gons)".to_string(), 0),
                        ("Triangles".to_string(), 1),
                    ],
                },
            ),
            Parameter::new(
                "preserve_hard_edges",
                "Preserve Hard Edges",
                "Settings",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "normal_strategy",
                "Normal Strategy",
                "Settings",
                ParameterValue::Int(0), // 0=Transfer
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Transfer from Input".to_string(), 0),
                        ("Recompute".to_string(), 1),
                    ],
                },
            ),
            Parameter::new(
                "tolerance",
                "Welding Tolerance",
                "Settings",
                ParameterValue::Float(1e-4),
                ParameterUIType::FloatSlider { min: 0.0, max: 0.1 },
            ),
            // --- Output Groups ---
            Parameter::new(
                "group_a_inside_b",
                "A Inside B Group",
                "Groups",
                ParameterValue::String("ainsideb".to_string()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "group_a_outside_b",
                "A Outside B Group",
                "Groups",
                ParameterValue::String("aoutsideb".to_string()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "group_b_inside_a",
                "B Inside A Group",
                "Groups",
                ParameterValue::String("binsidea".to_string()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "group_b_outside_a",
                "B Outside A Group",
                "Groups",
                ParameterValue::String("boutsidea".to_string()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "group_overlap",
                "Overlap Group",
                "Groups",
                ParameterValue::String("aboverlap".to_string()),
                ParameterUIType::String,
            ),
            // --- Output Edge Groups ---
            Parameter::new(
                "group_seam_aa",
                "A-A Seams",
                "Groups",
                ParameterValue::String("aseams".to_string()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "group_seam_bb",
                "B-B Seams",
                "Groups",
                ParameterValue::String("bseams".to_string()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "group_seam_ab",
                "A-B Seams",
                "Groups",
                ParameterValue::String("abseams".to_string()),
                ParameterUIType::String,
            ),
        ]
    }
}

/// Compute logic calling the kernel
pub fn compute_boolean(
    inputs: &[std::sync::Arc<Geometry>],
    params: &HashMap<String, ParameterValue>,
) -> Geometry {
    if inputs.len() < 2 {
        // Pass through if only 1 input, or empty if 0
        return if let Some(first) = inputs.first() {
            (**first).clone()
        } else {
            Geometry::new()
        };
    }

    let geo_a = &inputs[0];
    let geo_b = &inputs[1];

    // 1. Parse Parameters helper
    let get_float = |k: &str, def: f32| -> f32 {
        match params.get(k) {
            Some(ParameterValue::Float(f)) => *f,
            _ => def,
        }
    };

    let get_int = |k: &str, def: i32| -> i32 {
        match params.get(k) {
            Some(ParameterValue::Int(i)) => *i,
            _ => def,
        }
    };

    let get_bool = |k: &str, def: bool| -> bool {
        match params.get(k) {
            Some(ParameterValue::Bool(b)) => *b,
            _ => def,
        }
    };

    let welding_tolerance = get_float("tolerance", 1e-4);
    let op_idx = get_int("operation", 2);
    let subtract_mode = get_int("subtract_mode", 0);

    let output_topology = match get_int("output_topology", 0) {
        0 => OutputTopology::Polygons,
        _ => OutputTopology::Triangles,
    };

    let preserve_hard_edges = get_bool("preserve_hard_edges", true);

    let normal_strategy = match get_int("normal_strategy", 0) {
        0 => NormalStrategy::TransferFromInput,
        _ => NormalStrategy::Recompute,
    };

    // Handle empty inputs gracefully
    if geo_a.primitives().is_empty() {
        // If Union (0), return B. Else empty.
        return if op_idx == 0 {
            geo_b.as_ref().clone()
        } else {
            Geometry::new()
        };
    }
    if geo_b.primitives().is_empty() {
        // If Union (0) or Subtract A-B (2, mode 0), return A.
        // If Intersect (1), return empty.
        return if op_idx == 0 || (op_idx == 2 && subtract_mode == 0) {
            geo_a.as_ref().clone()
        } else {
            Geometry::new()
        };
    }

    // Prepare Inputs based on mode
    let (op, target, cutter) = match op_idx {
        0 => (BooleanOperation::Union, geo_a, geo_b),
        1 => (BooleanOperation::Intersection, geo_a, geo_b),
        2 => {
            match subtract_mode {
                0 => (BooleanOperation::Difference, geo_a, geo_b), // A - B
                1 => (BooleanOperation::Difference, geo_b, geo_a), // B - A
                2 => (BooleanOperation::Xor, geo_a, geo_b),        // Both (XOR)
                _ => (BooleanOperation::Difference, geo_a, geo_b),
            }
        }
        3 => (BooleanOperation::Xor, geo_a, geo_b), // Difference (XOR)
        _ => (BooleanOperation::Difference, geo_a, geo_b),
    };

    // Groups (TODO: Filter geometry by group before boolean)

    let settings = ManifoldBooleanSettings {
        output_topology,
        preserve_hard_edges,
        normal_strategy,
        welding_tolerance,
    };

    // 2. Call Kernel (Manifold Only)
    match run_manifold_boolean(target, cutter, op, &settings) {
        Ok(res) => res,
        Err(e) => {
            println!("Boolean Node Error (Manifold): {}. Returning empty.", e);
            Geometry::new()
        }
    }
}

// ----------------------------------------------------------------------------
// Node Registry Integration
// ----------------------------------------------------------------------------

impl NodeOp for BooleanNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        // Convert &[Parameter] to HashMap for our compute function
        let param_map = params
            .iter()
            .map(|p| (p.name.clone(), p.value.clone()))
            .collect();
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        Arc::new(compute_boolean(mats.as_slice(), &param_map))
    }
}

// Register with named ports for Boolean operations
crate::register_node!("Boolean", "Modeling", crate::nodes::modeling::boolean::boolean_node::BooleanNode;
    inputs: &["Geometry A", "Geometry B"], outputs: &["Output"],
    style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts);
