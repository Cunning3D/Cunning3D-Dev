//! Shared DCC reference grid plane (Editor + Player).
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use cunning_viewport::grid::grid_params::grid_params;
use cunning_viewport::viewport_options::{DisplayOptions, ViewportViewMode};
use cunning_viewport::MainCamera;

#[derive(Component)]
pub struct GridPlaneTag;

#[derive(Resource)]
pub struct GridPlaneHandles {
    pub material: Handle<GridPlaneMaterial>,
}

#[derive(Clone, Copy, ShaderType, Debug, Default)]
pub struct GridPlaneUniform {
    pub center: Vec2,
    pub base_step: f32,
    pub next_step: f32,
    pub blend: f32,
    pub minor_alpha: f32,
    pub major_alpha: f32,
    pub axis_alpha: f32,
    pub _pad0: f32,
    pub minor_rgb: Vec3,
    pub major_rgb: Vec3,
    pub axis_rgb: Vec3,
    pub _pad1: f32,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct GridPlaneMaterial {
    #[uniform(0)]
    pub u: GridPlaneUniform,
    #[texture(1)]
    #[sampler(2)]
    pub font_texture: Handle<Image>,
}

impl Material for GridPlaneMaterial {
    fn fragment_shader() -> ShaderRef { "shaders/cunning_grid_plane.wgsl".into() }
    fn alpha_mode(&self) -> AlphaMode { AlphaMode::Blend }
}

pub struct CunningGridPlanePlugin;

impl Plugin for CunningGridPlanePlugin {
    fn build(&self, app: &mut App) {
        // Ensure shader exists even on wasm (no asset filesystem).
        let render_h = {
            let mut shaders = app.world_mut().resource_mut::<Assets<Shader>>();
            shaders.add(Shader::from_wgsl(
                include_str!("../../../assets/shaders/cunning_grid_plane.wgsl"),
                "shaders/cunning_grid_plane.wgsl",
            ))
        };
        // Keep handle alive in main world.
        app.insert_resource(GridPlaneShader(render_h));

        app.add_plugins(MaterialPlugin::<GridPlaneMaterial>::default());
        app.add_systems(Startup, setup_grid_plane_system);
        app.add_systems(Update, update_grid_plane_system);
    }

    fn finish(&self, app: &mut App) {
        let render_h = app.world().resource::<GridPlaneShader>().0.clone();
        if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render_app.insert_resource(GridPlaneShader(render_h));
        }
    }
}

#[derive(Resource, Clone)]
pub struct GridPlaneShader(pub Handle<Shader>);

fn create_font_atlas(images: &mut Assets<Image>) -> Handle<Image> {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    use bevy_image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};

    // 12 chars: 0123456789-.
    let w = 512;
    let h = 384;
    let mut data = vec![0u8; (w * h) as usize];
    let font_5x7 = [
        0x3E, 0x51, 0x49, 0x45, 0x3E, // 0
        0x00, 0x42, 0x7F, 0x40, 0x00, // 1
        0x42, 0x61, 0x51, 0x49, 0x46, // 2
        0x21, 0x41, 0x45, 0x4B, 0x31, // 3
        0x18, 0x14, 0x12, 0x7F, 0x10, // 4
        0x27, 0x45, 0x45, 0x45, 0x39, // 5
        0x3C, 0x4A, 0x49, 0x49, 0x30, // 6
        0x01, 0x71, 0x09, 0x05, 0x03, // 7
        0x36, 0x49, 0x49, 0x49, 0x36, // 8
        0x06, 0x49, 0x49, 0x29, 0x1E, // 9
        0x08, 0x08, 0x08, 0x08, 0x08, // -
        0x00, 0x60, 0x60, 0x00, 0x00, // .
    ];
    for i in 0..12 {
        let cw = 128;
        let ch = 128;
        let col_idx = i % 4;
        let row_idx = i / 4;
        let ox = col_idx * cw;
        let oy = row_idx * ch;
        let char_bytes = &font_5x7[i * 5..(i + 1) * 5];
        let scale = 14;
        let scale_f = scale as f32;
        let margin_x = (cw - 5 * scale) / 2;
        let margin_y = (ch - 7 * scale) / 2;
        let ss: i32 = 4;
        for y in 0..ch {
            for x in 0..cw {
                if x < margin_x || x >= margin_x + 5 * scale || y < margin_y || y >= margin_y + 7 * scale { continue; }
                let mut hits: i32 = 0;
                for sy in 0..ss {
                    for sx in 0..ss {
                        let subx = (x - margin_x) as f32 + (sx as f32 + 0.5) / ss as f32;
                        let suby = (y - margin_y) as f32 + (sy as f32 + 0.5) / ss as f32;
                        let fx = (subx / scale_f).floor() as i32;
                        let fy = (suby / scale_f).floor() as i32;
                        if fx >= 0 && fx < 5 && fy >= 0 && fy < 7 {
                            let col_byte = char_bytes[fx as usize];
                            hits += ((col_byte >> (fy as u32)) & 1) as i32;
                        }
                    }
                }
                if hits > 0 {
                    let a = (hits as f32 / (ss * ss) as f32 * 255.0).round() as u8;
                    let p_idx = (oy + y) * w + (ox + x);
                    data[p_idx as usize] = a;
                }
            }
        }
    }
    let mut image = Image::new_fill(
        Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &data,
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    let mut desc = ImageSamplerDescriptor::default();
    desc.set_filter(ImageFilterMode::Linear);
    image.sampler = ImageSampler::Descriptor(desc);
    images.add(image)
}

pub fn setup_grid_plane_system(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GridPlaneMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    // Unit plane on XZ, centered at origin
    let mesh = meshes.add(Mesh::from(bevy::math::primitives::Plane3d::new(Vec3::Y, Vec2::splat(0.5))));
    let font_tex = create_font_atlas(&mut *images);
    let material = materials.add(GridPlaneMaterial {
        u: GridPlaneUniform {
            center: Vec2::ZERO,
            base_step: 1.0,
            next_step: 5.0,
            blend: 0.0,
            minor_alpha: 0.10,
            major_alpha: 0.30,
            axis_alpha: 0.50,
            minor_rgb: Vec3::splat(0.40),
            major_rgb: Vec3::splat(0.50),
            axis_rgb: Vec3::splat(0.60),
            ..default()
        },
        font_texture: font_tex,
    });
    commands.insert_resource(GridPlaneHandles { material: material.clone() });
    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_scale(Vec3::splat(100.0)),
        Visibility::Inherited,
        GridPlaneTag,
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

fn vp_size(camera: &Camera, win: Option<&Window>) -> Option<bevy_egui::egui::Vec2> {
    if let Some(r) = camera.logical_viewport_rect() {
        return Some(bevy_egui::egui::vec2(r.width(), r.height()));
    }
    if let Some(sz) = camera.logical_target_size() {
        return Some(bevy_egui::egui::vec2(sz.x, sz.y));
    }
    win.map(|w| bevy_egui::egui::vec2(w.width(), w.height()))
}

pub fn update_grid_plane_system(
    display_options: Res<DisplayOptions>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    windows: Query<&Window>,
    mut mats: ResMut<Assets<GridPlaneMaterial>>,
    handles: Option<Res<GridPlaneHandles>>,
    mut q_plane: Query<(&mut Transform, &mut Visibility), With<GridPlaneTag>>,
) {
    let Some(handles) = handles else { return; };
    let Ok((camera, camera_transform)) = camera_query.single() else { return; };
    let (mut tr, mut vis) = match q_plane.single_mut() { Ok(v) => v, Err(_) => return };
    if !display_options.grid.show || display_options.view_mode == ViewportViewMode::UV {
        *vis = Visibility::Hidden;
        return;
    }
    *vis = Visibility::Inherited;

    let win = windows.single().ok();
    let Some(vp) = vp_size(camera, win) else { return; };
    if vp.x <= 1.0 || vp.y <= 1.0 { return; }
    let Some(p) = grid_params(camera, camera_transform, vp, display_options.grid.major_target_px) else { return; };

    // Smooth blend between base step and next step (Houdini-like).
    let center = p.center;
    let ndc0 = camera.world_to_ndc(camera_transform, center);
    let refp = if ndc0.is_some() { center } else { Vec3::new(camera_transform.translation().x, 0.0, camera_transform.translation().z) };
    let (base_step, next_step, blend) = if let (Some(a), Some(b)) = (camera.world_to_ndc(camera_transform, refp), camera.world_to_ndc(camera_transform, refp + Vec3::X)) {
        let px0 = Vec2::new((a.x + 1.0) * 0.5 * vp.x, (1.0 - a.y) * 0.5 * vp.y);
        let px1 = Vec2::new((b.x + 1.0) * 0.5 * vp.x, (1.0 - b.y) * 0.5 * vp.y);
        let ppm = (px1 - px0).length().max(1e-4);
        let raw = (display_options.grid.major_target_px / ppm).max(1e-6);
        let log5 = raw.ln() / 5.0f32.ln();
        let base_pow = log5.floor();
        let frac = log5 - base_pow;
        let base_step = 5.0f32.powf(base_pow);
        let next_step = base_step * 5.0;
        let blend = (frac * 1.15 - 0.075).clamp(0.0, 1.0);
        let blend = blend * blend * (3.0 - 2.0 * blend);
        (base_step, next_step, blend)
    } else {
        (p.major_step, p.major_step * 5.0, 0.0)
    };

    // Plane transform: follow center and cover visible area.
    let size = (p.half_extent * 2.0).max(next_step * 8.0).min(5000.0);
    tr.translation = Vec3::new(p.center.x, 0.0, p.center.z);
    tr.scale = Vec3::new(size, 1.0, size);

    if let Some(m) = mats.get_mut(&handles.material) {
        m.u.center = Vec2::ZERO; // world anchored
        m.u.base_step = base_step;
        m.u.next_step = next_step;
        m.u.blend = blend;
        m.u._pad0 = if display_options.grid.show_labels { 1.0 } else { 0.0 };
        m.u.minor_alpha = 0.10;
        m.u.major_alpha = 0.22;
        m.u.axis_alpha = 0.30;
        m.u.minor_rgb = Vec3::splat(0.40);
        m.u.major_rgb = Vec3::splat(0.50);
        m.u.axis_rgb = Vec3::splat(0.60);
    }
}

