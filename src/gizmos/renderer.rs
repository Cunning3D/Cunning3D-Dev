use crate::gizmos::material::GizmoOverlayExt;
use bevy::asset::RenderAssetUsages;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::math::primitives::{Cuboid, Cylinder, Plane3d, Sphere};
use bevy::mesh::Indices;
use bevy::pbr::ExtendedMaterial;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
// In 0.13, Mesh::new uses PrimitiveTopology.
// RenderAssetUsages might be 0.14. In 0.13 it's default.
// Wait, 0.13 Mesh::new(topology).
// let mut mesh = Mesh::new(PrimitiveTopology::TriangleList);

pub use crate::cunning_core::traits::node_interface::{
    GizmoCommand, GizmoDrawBuffer, GizmoPrimitive,
};

// --- 1. Resources ---

#[derive(Resource)]
pub struct GizmoAssets {
    pub mesh_cylinder: Handle<Mesh>,
    pub mesh_cone: Handle<Mesh>,
    pub mesh_cube: Handle<Mesh>,
    pub mesh_sphere: Handle<Mesh>,
    pub mesh_plane: Handle<Mesh>,

    pub mat_red: Handle<GizmoMaterial>,
    pub mat_green: Handle<GizmoMaterial>,
    pub mat_blue: Handle<GizmoMaterial>,
    pub mat_yellow: Handle<GizmoMaterial>,
    pub mat_white: Handle<GizmoMaterial>,
    pub mat_gray: Handle<GizmoMaterial>,
    pub mat_transparent_red: Handle<GizmoMaterial>,
    pub mat_transparent_green: Handle<GizmoMaterial>,
    pub mat_transparent_blue: Handle<GizmoMaterial>,
}

// --- 2. Components ---

#[derive(Component)]
pub struct GizmoEntity;

pub type GizmoMaterial = ExtendedMaterial<StandardMaterial, GizmoOverlayExt>;

// --- 3. Systems ---

pub fn setup_gizmo_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GizmoMaterial>>,
) {
    // 1. Meshes (Using bevy::render::mesh::shape for 0.13 compatibility)

    // Cylinder: Height 1.0, Radius 0.05
    let mesh_cylinder = meshes.add(Mesh::from(Cylinder::new(0.05, 1.0)));

    // Cone: Manual generation
    let mesh_cone = meshes.add(create_cone_mesh(0.15, 0.4, 16));

    // Cube
    let mesh_cube = meshes.add(Mesh::from(Cuboid::from_size(Vec3::splat(1.0))));

    // Sphere
    let mesh_sphere = meshes.add(Mesh::from(Sphere::new(1.0)));

    // Plane (size 1.0)
    let mesh_plane = meshes.add(Mesh::from(Plane3d::new(Vec3::Y, Vec2::splat(0.5))));

    // 2. Materials (Lit + opaque, like DCC gizmos)
    // NOTE: large depth_bias will poison the depth buffer and can make wireframe depth-tests fail.
    let depth_bias = 0.0;

    let make_mat = |color: Color, alpha: f32| -> GizmoMaterial {
        let c = color.to_linear();
        GizmoMaterial {
            base: StandardMaterial {
                base_color: Color::linear_rgba(c.red * 2.5, c.green * 2.5, c.blue * 2.5, alpha),
                emissive: Color::linear_rgb(0.0, 0.0, 0.0).into(),
                perceptual_roughness: 0.0,
                reflectance: 0.0,
                alpha_mode: if alpha < 1.0 {
                    AlphaMode::Blend
                } else {
                    AlphaMode::Opaque
                },
                unlit: false,
                depth_bias,
                cull_mode: None,
                ..default()
            },
            extension: GizmoOverlayExt::default(),
        }
    };
    let (red, green, blue, yellow, gray) = (
        Color::srgb(1.0, 0.0, 0.0),
        Color::srgb(0.0, 1.0, 0.0),
        Color::srgb(0.0, 0.0, 1.0),
        Color::srgb(1.0, 1.0, 0.0),
        Color::srgb(0.5, 0.5, 0.5),
    );

    commands.insert_resource(GizmoAssets {
        mesh_cylinder,
        mesh_cone,
        mesh_cube,
        mesh_sphere,
        mesh_plane,
        mat_red: materials.add(make_mat(red, 1.0)),
        mat_green: materials.add(make_mat(green, 1.0)),
        mat_blue: materials.add(make_mat(blue, 1.0)),
        mat_yellow: materials.add(make_mat(yellow, 1.0)),
        mat_white: materials.add(make_mat(Color::WHITE, 1.0)),
        mat_gray: materials.add(make_mat(gray, 1.0)),
        mat_transparent_red: materials.add(make_mat(red, 0.4)),
        mat_transparent_green: materials.add(make_mat(green, 0.4)),
        mat_transparent_blue: materials.add(make_mat(blue, 0.4)),
    });
}

fn create_cone_mesh(radius: f32, height: f32, segments: usize) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    // Top tip
    positions.push([0.0, height * 0.5, 0.0]);
    normals.push([0.0, 1.0, 0.0]); // Approximate
    uvs.push([0.5, 1.0]);

    // Bottom center
    positions.push([0.0, -height * 0.5, 0.0]);
    normals.push([0.0, -1.0, 0.0]);
    uvs.push([0.5, 0.5]);

    let top_idx = 0;
    let bottom_center_idx = 1;
    let ring_start_idx = 2;

    for i in 0..=segments {
        let theta = (i as f32 / segments as f32) * std::f32::consts::TAU;
        let (sin, cos) = theta.sin_cos();
        let x = cos * radius;
        let z = sin * radius;

        // Side Vertex
        positions.push([x, -height * 0.5, z]);
        // Normal: (cos, height/radius, sin) normalized?
        let slope = radius / height;
        let n = Vec3::new(cos, slope, sin).normalize();
        normals.push([n.x, n.y, n.z]);
        uvs.push([i as f32 / segments as f32, 0.0]);
    }

    // Indices
    // Sides
    for i in 0..segments {
        let current = ring_start_idx + i;
        let next = ring_start_idx + i + 1;

        indices.push(top_idx as u32);
        indices.push(next as u32);
        indices.push(current as u32);
    }

    // Bottom Cap
    // We need vertices with down normal for cap?
    // For simplicity, reuse side vertices (shading might be weird at edge).
    // Or add duplicate vertices.
    // Let's just use the side vertices for now.
    for i in 0..segments {
        let current = ring_start_idx + i;
        let next = ring_start_idx + i + 1;

        indices.push(bottom_center_idx as u32);
        indices.push(current as u32);
        indices.push(next as u32);
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

pub fn sync_gizmo_entities(
    mut commands: Commands,
    mut buffer: ResMut<GizmoDrawBuffer>,
    assets: Res<GizmoAssets>,
    mut gizmos: Gizmos<crate::gizmos::TransformGizmoLines>, // For Line fallback (always on top)
    mut query: Query<
        (
            Entity,
            &mut Mesh3d,
            &mut MeshMaterial3d<GizmoMaterial>,
            &mut Transform,
            &mut Visibility,
        ),
        With<GizmoEntity>,
    >,
) {
    let mut iter = query.iter_mut();

    // 1. Update existing entities / Draw Lines
    for command in buffer.commands.iter() {
        match command {
            GizmoCommand::Mesh {
                primitive,
                transform,
                color,
            } => {
                if let Some((_e, mut mesh, mut mat, mut trans, mut vis)) = iter.next() {
                    *vis = Visibility::Inherited;
                    *trans = *transform;

                    // Update Mesh
                    *mesh = Mesh3d(match primitive {
                        GizmoPrimitive::Cylinder => assets.mesh_cylinder.clone(),
                        GizmoPrimitive::Cone => assets.mesh_cone.clone(),
                        GizmoPrimitive::Cube => assets.mesh_cube.clone(),
                        GizmoPrimitive::Sphere => assets.mesh_sphere.clone(),
                        GizmoPrimitive::Plane => assets.mesh_plane.clone(),
                    });

                    // Update Material
                    *mat = MeshMaterial3d(choose_material(&assets, color));
                } else {
                    // Spawn new entity
                    spawn_gizmo_entity(&mut commands, &assets, primitive, transform, color);
                }
            }
            GizmoCommand::Line { start, end, color } => {
                gizmos.line(*start, *end, *color);
            }
        }
    }

    // 2. Hide excess entities
    for (_e, _m, _mat, _t, mut vis) in iter {
        *vis = Visibility::Hidden;
    }

    // 3. Clear buffer for next frame
    buffer.clear();
}

fn spawn_gizmo_entity(
    commands: &mut Commands,
    assets: &GizmoAssets,
    primitive: &GizmoPrimitive,
    transform: &Transform,
    color: &Color,
) {
    let mesh = match primitive {
        GizmoPrimitive::Cylinder => assets.mesh_cylinder.clone(),
        GizmoPrimitive::Cone => assets.mesh_cone.clone(),
        GizmoPrimitive::Cube => assets.mesh_cube.clone(),
        GizmoPrimitive::Sphere => assets.mesh_sphere.clone(),
        GizmoPrimitive::Plane => assets.mesh_plane.clone(),
    };
    let material = choose_material(assets, color);

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        *transform,
        Visibility::Inherited,
        GizmoEntity,
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

fn choose_material(assets: &GizmoAssets, color: &Color) -> Handle<GizmoMaterial> {
    let c = color.to_srgba();
    if c.red > 0.9 && c.green < 0.1 && c.blue < 0.1 && c.alpha > 0.9 {
        assets.mat_red.clone()
    } else if c.red < 0.1 && c.green > 0.9 && c.blue < 0.1 && c.alpha > 0.9 {
        assets.mat_green.clone()
    } else if c.red < 0.1 && c.green < 0.1 && c.blue > 0.9 && c.alpha > 0.9 {
        assets.mat_blue.clone()
    } else if c.red > 0.9 && c.green > 0.9 && c.blue < 0.1 && c.alpha > 0.9 {
        assets.mat_yellow.clone()
    } else if c.red > 0.9 && c.green > 0.9 && c.blue > 0.9 && c.alpha > 0.9 {
        assets.mat_white.clone()
    } else if (0.45..0.55).contains(&c.red)
        && (0.45..0.55).contains(&c.green)
        && (0.45..0.55).contains(&c.blue)
        && c.alpha > 0.9
    {
        assets.mat_gray.clone()
    } else if c.red > 0.9 && c.alpha < 0.5 {
        assets.mat_transparent_red.clone()
    } else if c.green > 0.9 && c.alpha < 0.5 {
        assets.mat_transparent_green.clone()
    } else if c.blue > 0.9 && c.alpha < 0.5 {
        assets.mat_transparent_blue.clone()
    } else {
        assets.mat_white.clone()
    }
}
