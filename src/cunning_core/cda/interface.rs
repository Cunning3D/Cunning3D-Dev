//! CDA Input/Output Interface Definition
use crate::nodes::structs::NodeId;
use crate::nodes::PortId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct CDAInterface {
    pub id: Uuid,
    pub name: String,          // Port name (unique identifier)
    pub label: String,         // UI display label
    pub internal_node: NodeId, // Corresponding internal Input/Output node ID
    pub order: i32,            // Sort weight
}

impl CDAInterface {
    pub fn new(name: &str, internal_node: NodeId) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            label: name.to_string(),
            internal_node,
            order: 0,
        }
    }
    pub fn with_label(mut self, label: &str) -> Self {
        self.label = label.to_string();
        self
    }
    pub fn with_order(mut self, order: i32) -> Self {
        self.order = order;
        self
    }
    pub fn port_key(&self) -> PortId {
        PortId::from(format!("cda:{}", self.id).as_str())
    }
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum CDAInterfaceKind {
    Input,
    Output,
}
