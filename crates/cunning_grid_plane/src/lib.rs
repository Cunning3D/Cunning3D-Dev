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

/// SDF vector-rendered digit atlas (line-segment geometry per glyph).
fn create_font_atlas(images: &mut Assets<Image>) -> Handle<Image> {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    use bevy_image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};

    // 4×3 grid, 256px per cell → 1024×768 R8 SDF atlas
    let (w, h, cs) = (1024usize, 768usize, 256usize);
    let mut data = vec![0u8; w * h];

    // Rounded digit shapes: arcs for curves, lines for straights (5×7 char space, y-down)
    type S = (f32, f32, f32, f32);
    let pi = std::f32::consts::PI;
    let hp = pi * 0.5;
    let arc = |cx: f32, cy: f32, r: f32, a0: f32, a1: f32| -> Vec<S> {
        (0..8).map(|i| {
            let t0 = a0 + (a1 - a0) * i as f32 / 8.0;
            let t1 = a0 + (a1 - a0) * (i + 1) as f32 / 8.0;
            (cx + r * t0.cos(), cy + r * t0.sin(), cx + r * t1.cos(), cy + r * t1.sin())
        }).collect::<Vec<S>>()
    };
    let g: [Vec<S>; 12] = [
        // 0: Oval (two semicircles + sides)
        [arc(2.5, 1.8, 1.7, pi, 2.0*pi), vec![(4.2,1.8,4.2,5.2)],
         arc(2.5, 5.2, 1.7, 0.0, pi),    vec![(0.8,5.2,0.8,1.8)]].concat(),
        // 1: Vertical + flag + base serif
        vec![(2.5,0.5,2.5,6.5),(1.3,1.8,2.5,0.5),(1.5,6.5,3.5,6.5)],
        // 2: Top semicircle + diagonal + bottom bar
        [arc(2.5, 2.0, 1.7, pi, 2.0*pi),
         vec![(4.2,2.0,0.8,6.5),(0.8,6.5,4.2,6.5)]].concat(),
        // 3: Two C-bumps opening left
        [vec![(0.8,0.5,2.5,0.5)], arc(2.5, 2.0, 1.5, -hp, hp),
         vec![(2.5,3.5,1.8,3.5)], arc(2.5, 5.0, 1.5, -hp, hp),
         vec![(2.5,6.5,0.8,6.5)]].concat(),
        // 4: Left upper + middle + right full
        vec![(0.8,0.5,0.8,3.5),(0.8,3.5,4.2,3.5),(4.2,0.5,4.2,6.5)],
        // 5: Top bar + left upper + bottom 3/4 arc
        [vec![(4.2,0.5,0.8,0.5),(0.8,0.5,0.8,3.3),(0.8,3.3,2.5,3.3)],
         arc(2.5, 5.0, 1.7, -hp, pi)].concat(),
        // 6: Top arc + left side + bottom oval + right lower + mid bar
        [arc(2.5, 1.8, 1.7, pi, 2.0*pi), vec![(0.8,1.8,0.8,5.2)],
         arc(2.5, 5.2, 1.7, pi, 0.0), vec![(4.2,5.2,4.2,3.3),(0.8,3.3,4.2,3.3)]].concat(),
        // 7: Top bar + right side
        vec![(0.5,0.5,4.5,0.5),(4.5,0.5,4.5,6.5)],
        // 8: Two ovals stacked
        [arc(2.5, 1.8, 1.3, pi, 2.0*pi), vec![(3.8,1.8,3.8,3.5),(1.2,3.5,1.2,1.8)],
         vec![(1.2,3.5,3.8,3.5)],
         arc(2.5, 5.2, 1.7, pi, 0.0), vec![(4.2,5.2,4.2,3.5),(0.8,3.5,0.8,5.2)],
         arc(2.5, 5.2, 1.7, 0.0, pi)].concat(),
        // 9: Top oval + right side + bottom arc
        [arc(2.5, 1.8, 1.7, 0.0, pi), vec![(0.8,1.8,0.8,3.3)],
         vec![(0.8,3.3,4.2,3.3)],
         arc(2.5, 1.8, 1.7, pi, 2.0*pi), vec![(4.2,1.8,4.2,5.2)],
         arc(2.5, 5.2, 1.7, 0.0, pi)].concat(),
        // -: Short bar
        vec![(1.2,3.5,3.8,3.5)],
        // .: Small cross
        vec![(2.0,5.8,3.0,5.8),(2.5,5.3,2.5,6.3)],
    ];

    // Uniform scale: fit 5×7 char in square cell with margin
    let margin = cs as f32 * 0.08;
    let avail = cs as f32 - 2.0 * margin;
    let scale = avail / 7.0;
    let ox_base = (cs as f32 - 5.0 * scale) * 0.5;
    let oy_base = (cs as f32 - 7.0 * scale) * 0.5;
    let stroke = 14.0f32; // large stroke → round 14px-radius corner fillets
    let max_d = stroke + 8.0; // wide SDF encoding range for smooth gradients

    for (ci, segs) in g.iter().enumerate() {
        let (cx0, cy0) = ((ci % 4) * cs, (ci / 4) * cs);
        for py in 0..cs {
            for px in 0..cs {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let mut min_d = f32::MAX;
                for &(x0, y0, x1, y1) in segs {
                    min_d = min_d.min(seg_dist(fx, fy,
                        ox_base + x0 * scale, oy_base + y0 * scale,
                        ox_base + x1 * scale, oy_base + y1 * scale));
                }
                // SDF: 0.5 = edge, >0.5 = inside, <0.5 = outside
                let v = 0.5 + (stroke - min_d) / (2.0 * max_d);
                data[(cy0 + py) * w + (cx0 + px)] = (v.clamp(0.0, 1.0) * 255.0) as u8;
            }
        }
    }

    let mut image = Image::new_fill(
        Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        TextureDimension::D2, &data, TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    let mut desc = ImageSamplerDescriptor::default();
    desc.set_filter(ImageFilterMode::Linear); // SDF + bilinear = perfect AA at any scale
    image.sampler = ImageSampler::Descriptor(desc);
    images.add(image)
}

/// Point-to-segment distance in pixel space.
#[inline]
fn seg_dist(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let l2 = dx * dx + dy * dy;
    if l2 < 1e-10 { return ((px - x0).powi(2) + (py - y0).powi(2)).sqrt(); }
    let t = (((px - x0) * dx + (py - y0) * dy) / l2).clamp(0.0, 1.0);
    ((px - x0 - t * dx).powi(2) + (py - y0 - t * dy).powi(2)).sqrt()
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

