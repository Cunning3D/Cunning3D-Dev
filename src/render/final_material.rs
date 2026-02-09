use bevy::pbr::MaterialExtension;
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

pub type FinalMaterial = bevy::pbr::ExtendedMaterial<StandardMaterial, BackfaceTintExt>;

/// Backface tint for final mesh: restore the classic deep-blue backface look.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct BackfaceTintExt {
    /// xyz = tint rgb, w = strength [0..1]
    #[uniform(100)]
    pub tint_rgba: Vec4,
    /// xyz = multiply rgb, w = unused
    #[uniform(101)]
    pub mul_rgb: Vec4,
    /// x = voxel_size, y = line_width_px, z = enabled (0/1), w = reserved
    #[uniform(102)]
    pub voxel_grid_params: Vec4,
    /// xyz = grid rgb, w = grid alpha (0..1)
    #[uniform(103)]
    pub voxel_grid_color: Vec4,
}

impl Default for BackfaceTintExt {
    fn default() -> Self {
        Self {
            // Deep gray-blue (user-tuned look)
            tint_rgba: Vec4::new(0.10, 0.14, 0.28, 0.75),
            // Additional darken for backfaces
            mul_rgb: Vec4::new(0.55, 0.60, 0.75, 0.0),
            // Disabled by default (only enabled for pure-voxel geometries).
            voxel_grid_params: Vec4::new(0.1, 0.85, 0.0, 0.0),
            voxel_grid_color: Vec4::new(0.0, 0.0, 0.0, 0.55),
        }
    }
}

impl MaterialExtension for BackfaceTintExt {
    fn fragment_shader() -> ShaderRef {
        "shaders/cunning_pbr.wgsl".into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        "shaders/cunning_pbr.wgsl".into()
    }
}
