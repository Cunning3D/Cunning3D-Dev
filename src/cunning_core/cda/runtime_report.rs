use uuid::Uuid;

use crate::nodes::structs::NodeId;

#[derive(Clone, Debug)]
pub struct CdaRuntimeReport {
    pub instance_node_id: Option<NodeId>,
    pub asset_uuid: Uuid,
    pub asset_name: String,
    pub stage: &'static str,
    pub def_node_id: Option<NodeId>,
    pub op: Option<u32>,
    pub port: Option<String>,
    pub param: Option<String>,
    pub message: String,
}

impl std::fmt::Display for CdaRuntimeReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CDA {} asset={}({}) inst_node={:?} def_node={:?} op={:?} port={:?} param={:?} msg={}",
            self.stage,
            self.asset_name,
            self.asset_uuid,
            self.instance_node_id,
            self.def_node_id,
            self.op,
            self.port,
            self.param,
            self.message
        )
    }
}
