//! CDAAsset - 可参数化的节点图资产核心结构
use super::utils::apply_channel_value;
use super::{CDAInterface, PromotedParam};
use crate::nodes::parameter::ParameterValue;
use crate::nodes::structs::{NodeGraph, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;
use uuid::Uuid;

pub type CDAId = Uuid;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CdaHudUnit {
    pub node_id: NodeId,
    pub label: String,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CdaCoverlayUnit {
    pub node_id: NodeId,
    pub label: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub default_on: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CDAAsset {
    // 基础信息
    pub id: CDAId,
    pub name: String,
    pub version: u32,
    pub author: Option<String>,
    pub description: String,

    // 图相关
    pub inner_graph: NodeGraph,
    pub promoted_params: Vec<PromotedParam>,
    pub inputs: Vec<CDAInterface>,
    pub outputs: Vec<CDAInterface>,

    #[serde(default)]
    pub presets: Vec<CDAPreset>,

    // 默认视图状态
    pub view_center: Option<[f32; 2]>, // 默认平移中心
    pub view_zoom: Option<f32>,        // 默认缩放

    // 外观
    pub icon: Option<String>,    // 图标路径或emoji
    pub color: Option<[f32; 3]>, // 节点颜色RGB
    pub tags: Vec<String>,       // 标签列表

    // 帮助
    pub help_url: Option<String>,     // 外部文档地址
    pub help_content: Option<String>, // 内嵌Markdown说明

    // Overlay exposure (authoring-time): HUD is single-select, Coverlay is multi-select.
    #[serde(default)]
    pub hud_units: Vec<CdaHudUnit>,
    #[serde(default)]
    pub coverlay_units: Vec<CdaCoverlayUnit>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct CDAPreset {
    pub name: String,
    pub values: BTreeMap<String, ParameterValue>,
}

impl Default for CDAAsset {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "Untitled CDA".to_string(),
            version: 1,
            author: None,
            description: String::new(),
            inner_graph: NodeGraph::default(),
            promoted_params: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            presets: Vec::new(),
            view_center: None,
            view_zoom: None,
            icon: None,
            color: None,
            tags: Vec::new(),
            help_url: None,
            help_content: None,
            hud_units: Vec::new(),
            coverlay_units: Vec::new(),
        }
    }
}

impl CDAAsset {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Default::default()
        }
    }

    // ─────────────────────────── 参数管理 ───────────────────────────

    /// 添加提升参数
    pub fn add_promoted_param(&mut self, param: PromotedParam) {
        self.promoted_params.push(param);
    }

    /// 移除提升参数
    pub fn remove_promoted_param(&mut self, param_id: Uuid) {
        self.promoted_params.retain(|p| p.id != param_id);
    }

    /// 根据名称获取提升参数
    pub fn get_promoted_param(&self, name: &str) -> Option<&PromotedParam> {
        self.promoted_params.iter().find(|p| p.name == name)
    }

    /// 根据名称获取提升参数（可变）
    pub fn get_promoted_param_mut(&mut self, name: &str) -> Option<&mut PromotedParam> {
        self.promoted_params.iter_mut().find(|p| p.name == name)
    }

    /// 获取提升参数（按order和group排序）
    pub fn get_promoted_params_sorted(&self) -> Vec<&PromotedParam> {
        let mut params: Vec<_> = self.promoted_params.iter().collect();
        params.sort_by(|a, b| a.group.cmp(&b.group).then(a.order.cmp(&b.order)));
        params
    }

    /// 获取所有参数分组
    pub fn get_param_groups(&self) -> Vec<String> {
        let mut groups: Vec<_> = self
            .promoted_params
            .iter()
            .map(|p| p.group.clone())
            .collect();
        groups.sort();
        groups.dedup();
        groups
    }

    // ─────────────────────────── 输入/输出管理 ───────────────────────────

    /// 添加输入接口
    pub fn add_input(&mut self, iface: CDAInterface) {
        self.inputs.push(iface);
    }

    /// 添加输出接口
    pub fn add_output(&mut self, iface: CDAInterface) {
        self.outputs.push(iface);
    }

    /// 设置输入数量（自动创建/删除接口）
    pub fn set_input_count(&mut self, count: usize) {
        while self.inputs.len() < count {
            let idx = self.inputs.len();
            let id = Uuid::new_v4();
            self.inputs
                .push(CDAInterface::new(&format!("input_{}", idx), id).with_order(idx as i32));
        }
        self.inputs.truncate(count);
    }

    /// 设置输出数量
    pub fn set_output_count(&mut self, count: usize) {
        while self.outputs.len() < count {
            let idx = self.outputs.len();
            let id = Uuid::new_v4();
            self.outputs
                .push(CDAInterface::new(&format!("output_{}", idx), id).with_order(idx as i32));
        }
        self.outputs.truncate(count);
    }

    // ─────────────────────────── 参数值同步 ───────────────────────────

    /// 应用参数值到内部图（根据绑定关系）
    pub fn apply_param_values(&mut self, values: &HashMap<String, Vec<f64>>) {
        // 先收集所有需要应用的绑定
        let bindings: Vec<(NodeId, String, Option<usize>, f64)> = self
            .promoted_params
            .iter()
            .filter_map(|promoted| values.get(&promoted.name).map(|cv| (promoted, cv)))
            .flat_map(|(promoted, channel_values)| {
                promoted
                    .channels
                    .iter()
                    .enumerate()
                    .filter_map(|(ch_idx, channel)| {
                        channel_values.get(ch_idx).map(|&v| (channel, v))
                    })
                    .flat_map(|(channel, value)| {
                        channel.bindings.iter().map(move |b| {
                            (
                                b.target_node,
                                b.target_param.clone(),
                                b.target_channel,
                                value,
                            )
                        })
                    })
            })
            .collect();
        // 然后应用
        for (node_id, param_name, target_channel, value) in bindings {
            if let Some(node) = self.inner_graph.nodes.get_mut(&node_id) {
                if let Some(param) = node.parameters.iter_mut().find(|p| p.name == param_name) {
                    apply_channel_value(&mut param.value, target_channel, value);
                }
            }
        }
    }

    /// Normalize internal connection port names to stable keys (in:0/out:0/in:a/...) for runtime export.
    pub fn normalize_ports_for_runtime(&mut self) {
        use cunning_cda_runtime::registry::RuntimeRegistry;
        let reg = RuntimeRegistry::new_default();
        let type_by_id: HashMap<NodeId, String> = self
            .inner_graph
            .nodes
            .iter()
            .map(|(id, n)| (*id, n.node_type.type_id().to_string()))
            .collect();
        for c in self.inner_graph.connections.values_mut() {
            if let Some(t) = type_by_id.get(&c.from_node) {
                if let Some(op) = reg.op_code_for_type(t) {
                    if reg.port_id(op, false, c.from_port.as_str()).is_none() {
                        if let Some(k) = reg.port_key_by_label(op, false, c.from_port.as_str()) {
                            c.from_port = crate::nodes::PortId::from(k.as_str());
                        }
                    }
                }
            }
            if let Some(t) = type_by_id.get(&c.to_node) {
                if let Some(op) = reg.op_code_for_type(t) {
                    if reg.port_id(op, true, c.to_port.as_str()).is_none() {
                        if let Some(k) = reg.port_key_by_label(op, true, c.to_port.as_str()) {
                            c.to_port = crate::nodes::PortId::from(k.as_str());
                        }
                    }
                }
            }
        }
    }

    /// Apply promoted param channel default values into bound internal node parameters (editor preview).
    pub fn apply_promoted_defaults_to_inner(&mut self) {
        for p in &self.promoted_params {
            for ch in &p.channels {
                for b in &ch.bindings {
                    if let Some(node) = self.inner_graph.nodes.get_mut(&b.target_node) {
                        if let Some(param) = node
                            .parameters
                            .iter_mut()
                            .find(|pp| pp.name == b.target_param)
                        {
                            apply_channel_value(
                                &mut param.value,
                                b.target_channel,
                                ch.default_value,
                            );
                        }
                    }
                }
            }
        }
    }

    /// Get promoted link labels for a given internal node param (for UI highlight/tooltips).
    pub fn promoted_links_for_param(&self, node_id: NodeId, param_name: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in &self.promoted_params {
            for (src_ch, ch) in p.channels.iter().enumerate() {
                for b in &ch.bindings {
                    if b.target_node != node_id || b.target_param != param_name {
                        continue;
                    }
                    let dst = b
                        .target_channel
                        .map(|c| format!("ch{}", c))
                        .unwrap_or_else(|| "all".to_string());
                    out.push(format!("{}[{}]->{}", p.name, src_ch, dst));
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    pub fn count_invalid_bindings(&self) -> usize {
        let mut bad = 0usize;
        for p in &self.promoted_params {
            for ch in &p.channels {
                for b in &ch.bindings {
                    let Some(n) = self.inner_graph.nodes.get(&b.target_node) else {
                        bad += 1;
                        continue;
                    };
                    if !n.parameters.iter().any(|pp| pp.name == b.target_param) {
                        bad += 1;
                    }
                }
            }
        }
        bad
    }

    pub fn cleanup_invalid_bindings(&mut self) -> usize {
        let before = self
            .promoted_params
            .iter()
            .map(|p| p.total_bindings())
            .sum::<usize>();
        for p in &mut self.promoted_params {
            for ch in &mut p.channels {
                ch.bindings.retain(|b| {
                    self.inner_graph
                        .nodes
                        .get(&b.target_node)
                        .and_then(|n| n.parameters.iter().find(|pp| pp.name == b.target_param))
                        .is_some()
                });
            }
        }
        before.saturating_sub(
            self.promoted_params
                .iter()
                .map(|p| p.total_bindings())
                .sum::<usize>(),
        )
    }

    // ─────────────────────────── 内部节点查询 ───────────────────────────

    /// 列出内部所有节点（用于参数提升UI）
    pub fn list_internal_nodes(&self) -> Vec<(NodeId, &str, &str)> {
        self.inner_graph
            .nodes
            .iter()
            .map(|(id, node)| (*id, node.name.as_str(), node.node_type.name()))
            .collect()
    }

    /// 列出指定节点的所有参数（用于参数提升UI）
    pub fn list_node_params(&self, node_id: NodeId) -> Vec<(&str, &ParameterValue)> {
        self.inner_graph
            .nodes
            .get(&node_id)
            .map(|node| {
                node.parameters
                    .iter()
                    .map(|p| (p.name.as_str(), &p.value))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 检查参数是否已被绑定
    pub fn is_param_bound(
        &self,
        node_id: NodeId,
        param_name: &str,
        channel: Option<usize>,
    ) -> bool {
        self.promoted_params.iter().any(|p| {
            p.channels.iter().any(|ch| {
                ch.bindings.iter().any(|b| {
                    b.target_node == node_id
                        && b.target_param == param_name
                        && (channel.is_none() || b.target_channel == channel)
                })
            })
        })
    }

    /// 获取参数绑定到的提升参数名称
    pub fn get_binding_target(&self, node_id: NodeId, param_name: &str) -> Option<&str> {
        for p in &self.promoted_params {
            for ch in &p.channels {
                if ch
                    .bindings
                    .iter()
                    .any(|b| b.target_node == node_id && b.target_param == param_name)
                {
                    return Some(&p.name);
                }
            }
        }
        None
    }

    pub fn preset_names(&self) -> Vec<String> {
        let mut v: Vec<_> = self.presets.iter().map(|p| p.name.clone()).collect();
        v.sort();
        v
    }

    pub fn upsert_preset(
        &mut self,
        name: impl Into<String>,
        values: BTreeMap<String, ParameterValue>,
    ) {
        let name = name.into();
        if let Some(p) = self.presets.iter_mut().find(|p| p.name == name) {
            p.values = values;
            return;
        }
        self.presets.push(CDAPreset { name, values });
    }

    pub fn remove_preset(&mut self, name: &str) -> bool {
        let n0 = self.presets.len();
        self.presets.retain(|p| p.name != name);
        n0 != self.presets.len()
    }

    pub fn get_preset(&self, name: &str) -> Option<&CDAPreset> {
        self.presets.iter().find(|p| p.name == name)
    }
}
