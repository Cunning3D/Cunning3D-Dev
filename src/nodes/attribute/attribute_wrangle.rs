use crate::{
    cunning_core::scripting::api::register_api,
    cunning_core::traits::node_interface::{NodeOp, NodeParameters},
    libs::geometry::geo_ref::GeometryRef,
    mesh::Geometry,
    nodes::{
        parameter::{Parameter, ParameterUIType, ParameterValue},
        InputStyle, NodeStyle,
    },
    register_node,
};
use rhai::{Engine, Scope};
use std::sync::Arc;

#[derive(Default)]
pub struct AttributeWrangleNode;

impl NodeParameters for AttributeWrangleNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "code",
                "VEX Code",
                "Code",
                ParameterValue::String("// Example:\n// let pt_count = geo.point_count();\n// for i in 0..pt_count {\n//     let p = geo.get_point_pos(i);\n//     geo.set_point_pos(i, p + vec3(0.0, 1.0, 0.0));\n// }".to_string()),
                ParameterUIType::Code,
            ),
        ]
    }
}

impl NodeOp for AttributeWrangleNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let default_input = Arc::new(Geometry::new());
        let input_geo = mats.first().unwrap_or(&default_input);

        // Output starts as a clone of input
        let mut output_geo = (**input_geo).clone();

        // Get Code
        let code = params
            .iter()
            .find(|p| p.name == "code")
            .and_then(|p| match &p.value {
                ParameterValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default();

        if code.trim().is_empty() {
            return Arc::new(output_geo);
        }

        // Setup Rhai Engine
        let mut engine = Engine::new();
        register_api(&mut engine);

        let mut scope = Scope::new();

        // Inject `geo` into scope.
        // Note: In Rhai, to mutate an object in scope, we just use it.
        // When we extract it back, we get the modified version.
        scope.push("geo", output_geo);

        // Inject Helper Constants? (Maybe later)

        // Execute
        // We wrap in a block to print errors, but ideally this should log to a console in UI
        if let Err(e) = engine.run_with_scope(&mut scope, &code) {
            // For now, we just log to stdout.
            // In a real app, we'd attach this error to the node state.
            println!("Attribute Wrangle Error: {}", e);
            // On error, we might return the original input, or the partially modified one.
            // Let's return original to indicate failure visually (or partial if we can get it).
            // Usually returning original is safer to avoid broken geometry.
            return input_geo.clone();
        }

        // Retrieve modified geometry
        // scope.get_value returns a clone of the value in the scope.
        if let Some(modified_geo) = scope.get_value::<Geometry>("geo") {
            Arc::new(modified_geo)
        } else {
            // Should not happen unless script deleted 'geo' variable
            input_geo.clone()
        }
    }
}

register_node!("Attribute Wrangle", "Attribute", AttributeWrangleNode);

pub fn node_style() -> NodeStyle {
    NodeStyle::Normal
}

pub fn input_style() -> InputStyle {
    InputStyle::Individual
}
