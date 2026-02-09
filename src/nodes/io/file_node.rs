//! 万能文件导入节点 - 类似 Houdini File 节点
use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::importers::get_registry;

#[derive(Default)]
pub struct FileNode;

impl NodeParameters for FileNode {
    fn define_parameters() -> Vec<Parameter> {
        let exts = get_registry().supported_extensions();
        vec![Parameter::new(
            "file_path",
            "File",
            "File",
            ParameterValue::String(String::new()),
            ParameterUIType::FilePath {
                filters: exts.iter().map(|s| s.to_string()).collect(),
            },
        )]
    }
}

impl NodeOp for FileNode {
    fn compute(&self, params: &[Parameter], _inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let path_str = params
            .iter()
            .find(|p| p.name == "file_path")
            .and_then(|p| {
                if let ParameterValue::String(s) = &p.value {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        if path_str.is_empty() {
            return Arc::new(Geometry::new());
        }

        let path = resolve_import_path(&path_str);

        match get_registry().import(&path) {
            Ok(geo) => Arc::new(geo),
            Err(e) => {
                eprintln!("File Import Error: {} - Path: {}", e, path_str);
                Arc::new(Geometry::new())
            }
        }
    }
}

register_node!("File", "IO", FileNode);

#[inline]
fn resolve_import_path(path_str: &str) -> PathBuf {
    let p = Path::new(path_str);
    if p.is_absolute() { return p.to_path_buf(); }
    if let Ok(cwd) = std::env::current_dir() {
        let direct = cwd.join(p);
        if direct.exists() { return direct; }
        let assets = cwd.join("assets").join(p);
        if assets.exists() { return assets; }
    }
    p.to_path_buf()
}

pub fn node_style() -> crate::nodes::NodeStyle {
    crate::nodes::NodeStyle::Normal
}
pub fn input_style() -> crate::nodes::InputStyle {
    crate::nodes::InputStyle::Individual
}
