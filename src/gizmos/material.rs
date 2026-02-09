use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct GizmoOverlayExt {
    // Keep extension bindings away from StandardMaterial bindings.
    #[uniform(100)]
    pub _dummy: u32,
}

impl MaterialExtension for GizmoOverlayExt {
    fn alpha_mode() -> Option<AlphaMode> {
        Some(AlphaMode::Blend)
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let _ = (pipeline, layout, key);
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_write_enabled = false;
            ds.depth_compare = CompareFunction::Always;
        }
        Ok(())
    }
}
