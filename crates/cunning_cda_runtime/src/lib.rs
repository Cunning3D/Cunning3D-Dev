pub mod asset;
pub mod compiler;
pub mod error;
pub mod registry;
pub mod vm;

use std::collections::HashMap;
use std::sync::Arc;

use cunning_kernel::mesh::Geometry;
use cunning_kernel::traits::parameter::ParameterValue;

pub use asset::{ConnId, NodeId, RuntimeDefinition};
pub use vm::ExportValue;
pub type OpCode = u32;

pub fn compile(def: &RuntimeDefinition, reg: &registry::RuntimeRegistry) -> Result<compiler::ExecutionPlan, error::CdaCompileError> {
    compiler::compile(def, reg)
}

pub fn execute(plan: &compiler::ExecutionPlan, def: &RuntimeDefinition, reg: &registry::RuntimeRegistry, inputs: &[Arc<Geometry>], overrides: &HashMap<String, ParameterValue>, cancel: &std::sync::atomic::AtomicBool) -> Result<Vec<Arc<Geometry>>, error::CdaCookError> {
    vm::execute(plan, def, reg, inputs, overrides, cancel)
}

pub fn execute_selected(plan: &compiler::ExecutionPlan, def: &RuntimeDefinition, reg: &registry::RuntimeRegistry, inputs: &[Arc<Geometry>], overrides: &HashMap<String, ParameterValue>, cancel: &std::sync::atomic::AtomicBool, selected_exports: Option<&[String]>) -> Result<Vec<(String, ExportValue)>, error::CdaCookError> {
    vm::execute_selected(plan, def, reg, inputs, overrides, cancel, selected_exports)
}

/// Legacy convenience wrapper: compile + execute with default registry.
pub fn cook(def: &RuntimeDefinition, inputs: &[Arc<Geometry>], overrides: &HashMap<String, ParameterValue>, cancel: &std::sync::atomic::AtomicBool) -> Result<Vec<Arc<Geometry>>, error::CdaCookError> {
    let reg = registry::RuntimeRegistry::new_default();
    let plan = compile(def, &reg).map_err(|e| error::CdaCookError { asset_uuid: e.asset_uuid, asset_name: e.asset_name, node_id: None, op: None, port: None, param: None, kind: error::CdaCookErrorKind::Internal { message: format!("compile error: {:?}", e.kind) } })?;
    execute(&plan, def, &reg, inputs, overrides, cancel)
}

pub fn cook_selected(def: &RuntimeDefinition, inputs: &[Arc<Geometry>], overrides: &HashMap<String, ParameterValue>, cancel: &std::sync::atomic::AtomicBool, selected_exports: Option<&[String]>) -> Result<Vec<(String, ExportValue)>, error::CdaCookError> {
    let reg = registry::RuntimeRegistry::new_default();
    let plan = compile(def, &reg).map_err(|e| error::CdaCookError { asset_uuid: e.asset_uuid, asset_name: e.asset_name, node_id: None, op: None, port: None, param: None, kind: error::CdaCookErrorKind::Internal { message: format!("compile error: {:?}", e.kind) } })?;
    execute_selected(&plan, def, &reg, inputs, overrides, cancel, selected_exports)
}


