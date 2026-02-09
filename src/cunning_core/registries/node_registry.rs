use crate::cunning_core::traits::node_interface::{NodeInteraction, NodeOp};
use bevy::prelude::*;
use cunning_viewport::coverlay_dock::CoverlayPanelKind;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

// ----------------------------------------------------------------------------
// Static Descriptor (for Compile-time Rust Plugins via inventory)
// ----------------------------------------------------------------------------
pub type StaticNodeOpFactory = fn() -> Box<dyn NodeOp>;
pub type StaticNodeInteractionFactory = fn() -> Box<dyn NodeInteraction>;
pub type StaticParameterFactory = fn() -> Vec<crate::nodes::parameter::Parameter>;

/// Input port style
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputStyle {
    #[default]
    Single,
    Multi,
    NamedPorts,
}

/// Node display style
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NodeStyle {
    #[default]
    Normal,
    Large,
}

#[derive(Clone)]
pub struct StaticNodeDescriptor {
    pub name: &'static str,
    pub category: &'static str,
    pub op_factory: StaticNodeOpFactory,
    pub interaction_factory: Option<StaticNodeInteractionFactory>,
    /// Declarative coverlay panels exposed by this node type (editor-side).
    pub coverlay_kinds: &'static [CoverlayPanelKind],
    pub parameters_factory: Option<StaticParameterFactory>,
    pub inputs: &'static [&'static str],
    pub outputs: &'static [&'static str],
    pub input_style: InputStyle,
    pub node_style: NodeStyle,
}

// Tell inventory to collect these
inventory::collect!(StaticNodeDescriptor);

// ----------------------------------------------------------------------------
// Runtime Descriptor (for Dynamic DLLs and converted Static ones)
// ----------------------------------------------------------------------------
pub type RuntimeNodeOpFactory = Arc<dyn Fn() -> Box<dyn NodeOp> + Send + Sync>;
pub type RuntimeNodeInteractionFactory = Arc<dyn Fn() -> Box<dyn NodeInteraction> + Send + Sync>;
pub type RuntimeParameterFactory =
    Arc<dyn Fn() -> Vec<crate::nodes::parameter::Parameter> + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeOrigin {
    BuiltIn,
    Plugin,
}

#[derive(Clone)]
pub struct RuntimeNodeDescriptor {
    pub name: String,
    pub display_name: String,
    pub display_name_lc: String,
    pub category: String,
    pub op_factory: RuntimeNodeOpFactory,
    pub interaction_factory: Option<RuntimeNodeInteractionFactory>,
    /// Declarative coverlay panels exposed by this node type (editor-side).
    pub coverlay_kinds: Vec<CoverlayPanelKind>,
    pub parameters_factory: RuntimeParameterFactory,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub input_style: InputStyle,
    pub node_style: NodeStyle,
    pub origin: NodeOrigin,
}

// ----------------------------------------------------------------------------
// Registry
// ----------------------------------------------------------------------------
#[derive(Resource, Default, Clone)]
pub struct NodeRegistry {
    pub nodes: Arc<RwLock<HashMap<String, RuntimeNodeDescriptor>>>,
}

impl NodeRegistry {
    pub fn scan_and_load(&self) {
        let mut map = self.nodes.write().unwrap();
        for desc in inventory::iter::<StaticNodeDescriptor> {
            info!(
                "NodeRegistry: Discovered static node '{}' in category '{}'",
                desc.name, desc.category
            );

            // Convert Static to Runtime
            map.insert(
                desc.name.to_string(),
                RuntimeNodeDescriptor {
                    name: desc.name.to_string(),
                    display_name: desc.name.to_string(),
                    display_name_lc: desc.name.to_lowercase(),
                    category: desc.category.to_string(),
                    op_factory: Arc::new(desc.op_factory),
                    interaction_factory: desc.interaction_factory.map(|f| Arc::new(f) as _),
                    coverlay_kinds: desc.coverlay_kinds.to_vec(),
                    parameters_factory: desc
                        .parameters_factory
                        .map(|f| Arc::new(move || f()) as RuntimeParameterFactory)
                        .unwrap_or_else(|| Arc::new(|| Vec::new())),
                    inputs: desc.inputs.iter().map(|s| s.to_string()).collect(),
                    outputs: desc.outputs.iter().map(|s| s.to_string()).collect(),
                    input_style: desc.input_style,
                    node_style: desc.node_style,
                    origin: NodeOrigin::BuiltIn,
                },
            );
        }
    }

    pub fn register_dynamic(&self, mut desc: RuntimeNodeDescriptor) {
        let mut map = self.nodes.write().unwrap();
        info!(
            "NodeRegistry: Registering dynamic node '{}' in category '{}'",
            desc.name, desc.category
        );
        if desc.display_name_lc.is_empty() {
            desc.display_name_lc = desc.display_name.to_lowercase();
        }
        map.insert(desc.name.clone(), desc);
    }

    pub fn register_dynamic_node<T: NodeOp + Clone + 'static>(
        &self,
        name: &str,
        category: &str,
        op: Box<T>,
        params: Vec<crate::nodes::parameter::Parameter>,
    ) {
        // We need to create a factory that returns a NEW Box<dyn NodeOp> each time.
        // Since T is Clone, we can capture 'op' and clone it.
        // Note: 'op' is already boxed, we need the inner value to be clonable.
        // But Box<T> is not Clone unless T is Clone. T is Clone here.

        let op_cloneable = op.as_ref().clone();

        let op_factory = Arc::new(move || -> Box<dyn NodeOp> { Box::new(op_cloneable.clone()) });

        let params_clone = params.clone();
        let parameters_factory =
            Arc::new(move || -> Vec<crate::nodes::parameter::Parameter> { params_clone.clone() });

        let desc = RuntimeNodeDescriptor {
            name: name.to_string(),
            display_name: name.to_string(),
            display_name_lc: name.to_lowercase(),
            category: category.to_string(),
            op_factory,
            interaction_factory: None,
            coverlay_kinds: Vec::new(),
            parameters_factory,
            inputs: vec!["Input".to_string()],
            outputs: vec!["Output".to_string()],
            input_style: InputStyle::Single,
            node_style: NodeStyle::Normal,
            origin: NodeOrigin::BuiltIn,
        };

        self.register_dynamic(desc);
    }

    pub fn create_op(&self, name: &str) -> Option<Box<dyn NodeOp>> {
        let map = self.nodes.read().unwrap();
        map.get(name).map(|desc| (desc.op_factory)())
    }

    pub fn get_descriptor(&self, name: &str) -> Option<RuntimeNodeDescriptor> {
        self.nodes.read().unwrap().get(name).cloned()
    }

    pub fn get_parameters(&self, name: &str) -> Vec<crate::nodes::parameter::Parameter> {
        self.nodes
            .read()
            .unwrap()
            .get(name)
            .map(|d| (d.parameters_factory)())
            .unwrap_or_default()
    }

    pub fn list_categories(&self) -> Vec<String> {
        let map = self.nodes.read().unwrap();
        let mut cats: Vec<String> = map.values().map(|d| d.category.to_string()).collect();
        cats.sort();
        cats.dedup();
        cats
    }

    pub fn list_nodes_in_category(&self, category: &str) -> Vec<String> {
        let map = self.nodes.read().unwrap();
        let mut nodes: Vec<String> = map
            .values()
            .filter(|d| d.category == category)
            .map(|d| d.name.to_string())
            .collect();
        nodes.sort();
        nodes
    }

    pub fn list_nodes_in_category_ui(&self, category: &str) -> Vec<(String, String)> {
        let map = self.nodes.read().unwrap();
        let mut nodes: Vec<(String, String)> = map
            .values()
            .filter(|d| d.category == category)
            .map(|d| (d.display_name.clone(), d.name.clone()))
            .collect();
        nodes.sort_by(|a, b| a.0.cmp(&b.0));
        nodes
    }

    pub fn search_nodes_ui(&self, q: &str) -> Vec<(String, String)> {
        let q = q.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let map = self.nodes.read().unwrap();
        let mut out: Vec<(String, String)> = map
            .values()
            .filter_map(|d| {
                if d.display_name_lc.contains(&q) {
                    Some((d.display_name.clone(), d.name.clone()))
                } else {
                    None
                }
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn display_name(&self, key: &str) -> String {
        self.nodes
            .read()
            .unwrap()
            .get(key)
            .map(|d| d.display_name.clone())
            .unwrap_or_else(|| key.to_string())
    }
}

#[macro_export]
macro_rules! register_node {
    // Basic version: single input single output
    ($name:expr, $category:expr, $op_type:ty) => {
        inventory::submit! {
            $crate::cunning_core::registries::node_registry::StaticNodeDescriptor {
                name: $name,
                category: $category,
                op_factory: || Box::new(<$op_type>::default()),
                interaction_factory: None,
                coverlay_kinds: &[],
                parameters_factory: Some(|| <$op_type as $crate::cunning_core::traits::node_interface::NodeParameters>::define_parameters()),
                inputs: &["Input"],
                outputs: &["Output"],
                input_style: $crate::cunning_core::registries::node_registry::InputStyle::Single,
                node_style: $crate::cunning_core::registries::node_registry::NodeStyle::Normal,
            }
        }
    };
    // Interactive version
    ($name:expr, $category:expr, $op_type:ty, $inter_type:ty) => {
        inventory::submit! {
            $crate::cunning_core::registries::node_registry::StaticNodeDescriptor {
                name: $name,
                category: $category,
                op_factory: || Box::new(<$op_type>::default()),
                interaction_factory: Some(|| Box::new(<$inter_type>::default())),
                coverlay_kinds: &[],
                parameters_factory: Some(|| <$op_type as $crate::cunning_core::traits::node_interface::NodeParameters>::define_parameters()),
                inputs: &["Input"],
                outputs: &["Output"],
                input_style: $crate::cunning_core::registries::node_registry::InputStyle::Single,
                node_style: $crate::cunning_core::registries::node_registry::NodeStyle::Normal,
            }
        }
    };
    // Basic version (declares Coverlay panel): single input single output
    ($name:expr, $category:expr, $op_type:ty; coverlay: $coverlay_kinds:expr) => {
        inventory::submit! {
            $crate::cunning_core::registries::node_registry::StaticNodeDescriptor {
                name: $name,
                category: $category,
                op_factory: || Box::new(<$op_type>::default()),
                interaction_factory: None,
                coverlay_kinds: $coverlay_kinds,
                parameters_factory: Some(|| <$op_type as $crate::cunning_core::traits::node_interface::NodeParameters>::define_parameters()),
                inputs: &["Input"],
                outputs: &["Output"],
                input_style: $crate::cunning_core::registries::node_registry::InputStyle::Single,
                node_style: $crate::cunning_core::registries::node_registry::NodeStyle::Normal,
            }
        }
    };
    // Full version: specify ports/style
    ($name:expr, $category:expr, $op_type:ty; inputs: $inputs:expr, outputs: $outputs:expr, style: $style:expr) => {
        inventory::submit! {
            $crate::cunning_core::registries::node_registry::StaticNodeDescriptor {
                name: $name,
                category: $category,
                op_factory: || Box::new(<$op_type>::default()),
                interaction_factory: None,
                coverlay_kinds: &[],
                parameters_factory: Some(|| <$op_type as $crate::cunning_core::traits::node_interface::NodeParameters>::define_parameters()),
                inputs: $inputs,
                outputs: $outputs,
                input_style: $style,
                node_style: $crate::cunning_core::registries::node_registry::NodeStyle::Normal,
            }
        }
    };
    // Full version (interactive): specify ports/style
    ($name:expr, $category:expr, $op_type:ty, $inter_type:ty; inputs: $inputs:expr, outputs: $outputs:expr, style: $style:expr) => {
        inventory::submit! {
            $crate::cunning_core::registries::node_registry::StaticNodeDescriptor {
                name: $name,
                category: $category,
                op_factory: || Box::new(<$op_type>::default()),
                interaction_factory: Some(|| Box::new(<$inter_type>::default())),
                coverlay_kinds: &[],
                parameters_factory: Some(|| <$op_type as $crate::cunning_core::traits::node_interface::NodeParameters>::define_parameters()),
                inputs: $inputs,
                outputs: $outputs,
                input_style: $style,
                node_style: $crate::cunning_core::registries::node_registry::NodeStyle::Normal,
            }
        }
    };
}
