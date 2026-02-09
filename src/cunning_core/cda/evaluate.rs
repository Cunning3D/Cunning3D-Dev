//! CDA evaluation logic
use super::CDAAsset;
use super::CDAId;
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::mesh::Geometry;
use crate::nodes::parameter::ParameterValue;
use crate::nodes::structs::NodeId;
use crate::nodes::structs::{NodeGraph, NodeParamOverrides};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

thread_local! { static CDA_EVAL_STACK: RefCell<Vec<CDAId>> = RefCell::new(Vec::new()); }
thread_local! { static CDA_SIMPLE_CACHE: RefCell<HashMap<CDAId, (u64, NodeGraph)>> = RefCell::new(HashMap::new()); }

impl CDAAsset {
    #[inline]
    fn apply_value_overrides(
        graph: &mut NodeGraph,
        ovs: &HashMap<NodeId, HashMap<String, ParameterValue>>,
    ) -> Vec<(NodeId, usize, ParameterValue)> {
        let mut undo = Vec::new();
        for (nid, m) in ovs {
            let Some(n) = graph.nodes.get_mut(nid) else {
                continue;
            };
            for (i, p) in n.parameters.iter_mut().enumerate() {
                if let Some(v) = m.get(&p.name) {
                    undo.push((*nid, i, p.value.clone()));
                    p.value = v.clone();
                }
            }
        }
        undo
    }
    #[inline]
    fn restore_value_overrides(graph: &mut NodeGraph, undo: Vec<(NodeId, usize, ParameterValue)>) {
        for (nid, i, v) in undo {
            if let Some(n) = graph.nodes.get_mut(&nid) {
                if let Some(p) = n.parameters.get_mut(i) {
                    p.value = v;
                }
            }
        }
    }

    /// Evaluate CDA with channel value overrides
    /// - channel_values: Exposed parameter name -> channel value vectors
    /// - input_geometries: Input geometries
    /// - registry: Node registry
    pub fn evaluate_into(
        &self,
        channel_values: &HashMap<String, Vec<f64>>,
        input_geometries: &[Arc<Geometry>],
        registry: &NodeRegistry,
        graph: &mut NodeGraph,
    ) -> Vec<Arc<Geometry>> {
        self.evaluate_into_with_value_overrides(
            channel_values,
            input_geometries,
            registry,
            graph,
            None,
        )
    }

    pub fn evaluate_into_with_value_overrides(
        &self,
        channel_values: &HashMap<String, Vec<f64>>,
        input_geometries: &[Arc<Geometry>],
        registry: &NodeRegistry,
        graph: &mut NodeGraph,
        value_overrides: Option<&HashMap<NodeId, HashMap<String, ParameterValue>>>,
    ) -> Vec<Arc<Geometry>> {
        let cycle = CDA_EVAL_STACK.with(|s| {
            let mut st = s.borrow_mut();
            if st.contains(&self.id) {
                true
            } else {
                st.push(self.id);
                false
            }
        });
        if cycle {
            eprintln!("CDA cycle detected: {}", self.name);
            return self
                .outputs
                .iter()
                .map(|_| Arc::new(Geometry::new()))
                .collect();
        }
        struct PopGuard(CDAId);
        impl Drop for PopGuard {
            fn drop(&mut self) {
                CDA_EVAL_STACK.with(|s| {
                    let mut st = s.borrow_mut();
                    if let Some(i) = st.iter().rposition(|x| *x == self.0) {
                        st.remove(i);
                    }
                });
            }
        }
        let _guard = PopGuard(self.id);

        graph.geometry_cache.clear();
        graph.port_geometry_cache.clear();
        graph.dirty_tracker.clear();
        graph.final_geometry = Arc::new(Geometry::new());

        let undo = value_overrides
            .map(|o| Self::apply_value_overrides(graph, o))
            .unwrap_or_default();

        let mut invalid = 0usize;
        let mut overrides: NodeParamOverrides = HashMap::new();
        for promoted in &self.promoted_params {
            let override_vals = channel_values.get(&promoted.name);
            for (ch_idx, channel) in promoted.channels.iter().enumerate() {
                let val = override_vals
                    .and_then(|v| v.get(ch_idx))
                    .copied()
                    .unwrap_or(channel.default_value);
                for b in &channel.bindings {
                    let Some(n) = graph.nodes.get(&b.target_node) else {
                        invalid += 1;
                        continue;
                    };
                    if n.parameters.iter().any(|p| p.name == b.target_param) {
                        overrides.entry(b.target_node).or_default().push((
                            b.target_param.clone(),
                            b.target_channel,
                            val,
                        ));
                    } else {
                        invalid += 1;
                    }
                }
            }
        }
        if invalid > 0 {
            eprintln!(
                "CDA evaluate: {} invalid bindings in {}",
                invalid, self.name
            );
        }

        for (i, iface) in self.inputs.iter().enumerate() {
            if let Some(geo) = input_geometries.get(i) {
                graph
                    .geometry_cache
                    .insert(iface.internal_node, geo.clone());
            }
        }

        let targets: HashSet<NodeId> = self.outputs.iter().map(|o| o.internal_node).collect();
        graph.compute_with_overrides(&targets, registry, None, &overrides);
        let outs = self
            .outputs
            .iter()
            .map(|o| {
                graph
                    .geometry_cache
                    .get(&o.internal_node)
                    .cloned()
                    .unwrap_or_else(|| Arc::new(Geometry::new()))
            })
            .collect();
        if !undo.is_empty() {
            Self::restore_value_overrides(graph, undo);
        }
        outs
    }

    /// Fast evaluation (single input single output)
    pub fn evaluate_simple(
        &self,
        channel_values: &HashMap<String, Vec<f64>>,
        input: Option<Arc<Geometry>>,
        registry: &NodeRegistry,
    ) -> Arc<Geometry> {
        let inputs = input.map(|g| vec![g]).unwrap_or_default();
        fn sig(g: &NodeGraph) -> u64 {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::Hasher;
            let mut n = 0u64;
            for (id, node) in &g.nodes {
                let mut h = DefaultHasher::new();
                h.write(id.as_bytes());
                h.write(node.node_type.name().as_bytes());
                n = n.wrapping_add(h.finish());
            }
            let mut e = 0u64;
            for (id, c) in &g.connections {
                let mut h = DefaultHasher::new();
                h.write(id.as_bytes());
                h.write(c.from_node.as_bytes());
                h.write(c.to_node.as_bytes());
                h.write(c.from_port.as_bytes());
                h.write(c.to_port.as_bytes());
                e = e.wrapping_add(h.finish());
            }
            (g.nodes.len() as u64)
                .wrapping_mul(1315423911)
                .wrapping_add(n)
                ^ (g.connections.len() as u64)
                    .wrapping_mul(2654435761)
                    .wrapping_add(e)
        }
        let fp = sig(&self.inner_graph);
        CDA_SIMPLE_CACHE.with(|c| {
            let mut m = c.borrow_mut();
            let entry = m
                .entry(self.id)
                .or_insert_with(|| (fp, self.inner_graph.clone()));
            if entry.0 != fp {
                *entry = (fp, self.inner_graph.clone());
            }
            self.evaluate_into(channel_values, &inputs, registry, &mut entry.1)
                .into_iter()
                .next()
                .unwrap_or_else(|| Arc::new(Geometry::new()))
        })
    }

    /// Cached multi-output evaluation (editor path): same cache as `evaluate_simple`, but returns all outputs.
    pub fn evaluate_outputs_cached(
        &self,
        channel_values: &HashMap<String, Vec<f64>>,
        inputs: &[Arc<Geometry>],
        registry: &NodeRegistry,
    ) -> Vec<Arc<Geometry>> {
        self.evaluate_outputs_cached_with_value_overrides(channel_values, inputs, registry, None)
    }

    pub fn evaluate_outputs_cached_with_value_overrides(
        &self,
        channel_values: &HashMap<String, Vec<f64>>,
        inputs: &[Arc<Geometry>],
        registry: &NodeRegistry,
        value_overrides: Option<&HashMap<NodeId, HashMap<String, ParameterValue>>>,
    ) -> Vec<Arc<Geometry>> {
        fn sig(g: &NodeGraph) -> u64 {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::Hasher;
            let mut n = 0u64;
            for (id, node) in &g.nodes {
                let mut h = DefaultHasher::new();
                h.write(id.as_bytes());
                h.write(node.node_type.name().as_bytes());
                n = n.wrapping_add(h.finish());
            }
            let mut e = 0u64;
            for (id, c) in &g.connections {
                let mut h = DefaultHasher::new();
                h.write(id.as_bytes());
                h.write(c.from_node.as_bytes());
                h.write(c.to_node.as_bytes());
                h.write(c.from_port.as_bytes());
                h.write(c.to_port.as_bytes());
                e = e.wrapping_add(h.finish());
            }
            (g.nodes.len() as u64)
                .wrapping_mul(1315423911)
                .wrapping_add(n)
                ^ (g.connections.len() as u64)
                    .wrapping_mul(2654435761)
                    .wrapping_add(e)
        }
        let fp = sig(&self.inner_graph);
        CDA_SIMPLE_CACHE.with(|c| {
            let mut m = c.borrow_mut();
            let entry = m
                .entry(self.id)
                .or_insert_with(|| (fp, self.inner_graph.clone()));
            if entry.0 != fp {
                *entry = (fp, self.inner_graph.clone());
            }
            self.evaluate_into_with_value_overrides(
                channel_values,
                inputs,
                registry,
                &mut entry.1,
                value_overrides,
            )
        })
    }
}
