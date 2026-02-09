//! IO 节点模块 - 文件导入导出
pub mod file_node;
pub mod importers;

/// Legacy alias for FBX importer node
pub mod fbx_importer {
    use crate::cunning_core::traits::node_interface::NodeParameters;
    use crate::libs::geometry::mesh::Geometry;
    use crate::nodes::io::importers::FileImporter;
    use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};

    pub struct FbxImporterNode;

    impl NodeParameters for FbxImporterNode {
        fn define_parameters() -> Vec<Parameter> {
            vec![Parameter::new(
                "path",
                "File Path",
                "Main",
                ParameterValue::String(String::new()),
                ParameterUIType::String,
            )]
        }
    }

    pub fn compute_fbx_import(
        params: &std::collections::HashMap<String, ParameterValue>,
    ) -> Geometry {
        let path = params
            .get("path")
            .and_then(|p| {
                if let ParameterValue::String(s) = p {
                    Some(s.as_str())
                } else {
                    None
                }
            })
            .unwrap_or("");
        if path.is_empty() {
            return Geometry::new();
        }
        super::importers::fbx::FbxImporter
            .import(std::path::Path::new(path))
            .unwrap_or_else(|_| Geometry::new())
    }
}
