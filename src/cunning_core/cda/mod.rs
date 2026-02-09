//! CDA (Cunning Digital Asset) - 可参数化的节点图资产，类似Houdini HDA但更灵活
pub mod asset;
pub mod evaluate;
pub mod interface;
pub mod library;
pub mod promoted_param;
pub mod runtime_report;
pub mod serialization;
pub mod utils;

pub use asset::{CDAAsset, CDAId, CDAPreset};
pub use interface::{CDAInterface, CDAInterfaceKind};
pub use library::{CdaAssetRef, CdaLibrary};
pub use promoted_param::{
    DropdownItem, ParamBinding, ParamChannel, ParamUIConfig, PromotedParam, PromotedParamType,
};
pub use serialization::CDAError;
pub(crate) use utils::{promoted_channels_to_value, promoted_type_to_ui};
