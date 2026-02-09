use std::sync::Arc;

use cunning_kernel::mesh::Geometry;
use cunning_kernel::traits::parameter::ParameterValue;

use cunning_cda_runtime::asset::{NodeId, RuntimeDefinition};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HostCommand {
    LoadCdaUrl { url: String },
    LoadCdaBytes { bytes: Vec<u8> }, // native/tests
    SetOverride { name: String, value: ParameterValue },
    /// Override an internal node parameter by node id (for high-interaction coverlay tools).
    SetInternalOverride {
        node: NodeId,
        param: String,
        value: ParameterValue,
    },
    /// Apply a list of commands in order (batching for high-frequency tools).
    Batch { cmds: Vec<HostCommand> },
    Cook,
}

#[derive(Clone, Debug)]
pub enum WorkerEvent {
    Error { message: String },
    AssetReady { def: Arc<RuntimeDefinition> },
    CookFinished { duration_ms: u32, outputs: Vec<Arc<Geometry>> },
}

/// Worker-wire event for wasm32 WebWorker transport.
/// Uses JSON bytes for `RuntimeDefinition` and cooked `Geometry` outputs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WorkerWireEvent {
    Error { message: String },
    AssetReady { def_json: Vec<u8> },
    CookFinished { duration_ms: u32, outputs_json: Vec<Vec<u8>> },
}