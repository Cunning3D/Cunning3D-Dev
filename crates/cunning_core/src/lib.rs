// Unity/Runtime core: Geometry kernel + core operators. No editor tabs/UI modules.

// Allow shared node code to reference this crate as `cunning_kernel`.
extern crate self as cunning_kernel;

#[path = "../../../src/cunning_core/core/geometry/mod.rs"]
pub mod geometry;

#[path = "../../../src/cunning_core/core/algorithms/mod.rs"]
pub mod algorithms;

pub mod io;
pub mod spline_snapshot_fbs;
pub mod coord;

pub mod libs {
    pub use crate::geometry as geometry;
    pub use crate::algorithms as algorithms;
}

#[path = "../../../src/mesh.rs"]
pub mod mesh;

pub mod volume { pub use crate::geometry::volume::*; }

#[path = "../../../src/cunning_core/traits/parameter.rs"]
pub mod parameter;

pub mod cunning_core {
    pub mod core {
        pub mod algorithms { pub use crate::algorithms::*; }
        pub mod geometry { pub use crate::geometry::*; }
    }

    pub mod traits {
        pub mod node_interface {
            use std::any::{Any, TypeId};
            use std::sync::Arc;
            use uuid::Uuid;
            use crate::mesh::Geometry;
            use crate::geometry::geo_ref::GeometryRef;
            use crate::parameter::Parameter;

            pub trait NodeOp: Send + Sync {
                fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry>;
            }

            pub trait NodeParameters { fn define_parameters() -> Vec<Parameter>; }

            pub trait ServiceProvider { fn get_service(&self, service_type: TypeId) -> Option<&dyn Any>; }
            impl dyn ServiceProvider + '_ {
                pub fn get<T: 'static>(&self) -> Option<&T> {
                    self.get_service(TypeId::of::<T>()).and_then(|s| s.downcast_ref::<T>())
                }
            }

            pub trait NodeInteraction: Send + Sync {
                fn draw_hud(&self, _ui: &mut (), _services: &dyn ServiceProvider, _node_id: Uuid) {}
            }
        }

        pub use crate::parameter as parameter;
    }
}

pub mod traits {
    pub use crate::parameter as parameter;
}

#[path = "../../../src/nodes/group/utils.rs"]
mod node_group_utils;

#[path = "../../../src/nodes/group/group_create.rs"]
mod node_group_create;

#[path = "../../../src/nodes/modeling/boolean/boolean_node.rs"]
mod node_boolean;

#[path = "../../../src/nodes/modeling/poly_extrude.rs"]
mod node_poly_extrude;

#[path = "../../../src/nodes/modeling/poly_bevel/mod.rs"]
mod node_poly_bevel;

#[path = "../../../src/nodes/voxel/voxel_edit.rs"]
mod node_voxel_edit;

pub mod nodes {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum NodeStyle { Normal }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum InputStyle { Individual }

    pub mod parameter {
        pub use crate::parameter::{Parameter, ParameterId, ParameterUIType, ParameterValue};
    }

    pub mod group {
        pub mod group_create { pub use crate::node_group_create::*; }
        pub mod utils { pub use crate::node_group_utils::*; }
    }

    pub mod modeling {
        pub mod boolean { pub use crate::node_boolean::*; }
        pub mod poly_extrude { pub use crate::node_poly_extrude::*; }
        pub mod poly_bevel { pub use crate::node_poly_bevel::*; }
    }

    pub mod voxel {
        pub mod voxel_edit { pub use crate::node_voxel_edit::*; }
    }
}

#[macro_export]
macro_rules! register_node { ($($tt:tt)*) => {}; }

