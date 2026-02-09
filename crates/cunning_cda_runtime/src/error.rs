use uuid::Uuid;

use crate::{NodeId, OpCode};

#[derive(Clone, Debug)]
pub struct CdaCompileError {
    pub asset_uuid: Uuid,
    pub asset_name: String,
    pub kind: CdaCompileErrorKind,
}

#[derive(Clone, Debug)]
pub enum CdaCompileErrorKind {
    UnknownOp { node_id: NodeId, type_id: String },
    MissingNode { node_id: NodeId },
    MissingPort { node_id: NodeId, port: String, is_input: bool },
    MissingOutputs,
    OutputBindingMissing { output_name: String },
    MultiWireViolation { node_id: NodeId, port: String, count: usize },
    CycleDetected { nodes: Vec<NodeId> },
}

#[derive(Clone, Debug)]
pub struct CdaCookError {
    pub asset_uuid: Uuid,
    pub asset_name: String,
    pub node_id: Option<NodeId>,
    pub op: Option<OpCode>,
    pub port: Option<String>,
    pub param: Option<String>,
    pub kind: CdaCookErrorKind,
}

#[derive(Clone, Debug)]
pub enum CdaCookErrorKind {
    Cancelled,
    MissingDependency { src_node: NodeId },
    OpFailed { message: String },
    Internal { message: String },
}

