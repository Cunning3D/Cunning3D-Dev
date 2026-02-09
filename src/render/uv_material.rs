use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

#[derive(Resource, Clone)]
pub struct GlobalUvMaterial(pub Handle<UvMaterial>);

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct UvMaterial {}

impl Material for UvMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/uv_material.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/uv_material.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}

pub fn setup_uv_material(mut commands: Commands, mut materials: ResMut<Assets<UvMaterial>>) {
    commands.insert_resource(GlobalUvMaterial(materials.add(UvMaterial {})));
}
