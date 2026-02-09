//! Shared viewport display options & camera view messages (Editor + Player).

use bevy::prelude::{Color, Message, Quat, Resource};

#[derive(Message, Default)]
pub struct OpenNaiveWindowEvent;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DisplayMode {
    Shaded,
    Wireframe,
    ShadedAndWireframe,
}

#[derive(Debug, PartialEq, Clone, Copy, Default)]
pub enum ViewportViewMode {
    #[default]
    Perspective,
    Orthographic,
    UV,
    NodeImage,
}

#[derive(Message, Clone, Copy, Debug)]
pub enum CameraViewDirection {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    Perspective,
    Custom(bevy::prelude::Vec3),
}

#[derive(Message)]
pub struct SetCameraViewEvent(pub CameraViewDirection);

#[derive(Message)]
pub struct CameraRotateEvent {
    pub rotation: Quat,
    pub immediate: bool,
}

#[derive(Debug)]
pub struct GridSettings {
    pub show: bool,
    pub show_labels: bool,
    pub major_target_px: f32,
}

#[derive(Debug)]
pub struct OverlaySettings {
    pub show_points: bool,
    pub show_point_numbers: bool,
    pub show_vertex_numbers: bool,
    pub show_vertex_normals: bool,
    pub show_primitive_numbers: bool,
    pub show_primitive_normals: bool,
    pub normal_length: f32,
    pub normal_color: Color,
    pub voxel_grid_line_px: f32,
    pub point_group_viz: Option<String>,
    pub edge_group_viz: Option<String>,
    pub vertex_group_viz: Option<String>,
    pub highlight_active_group: bool,
}

#[derive(Debug)]
pub struct TurntableSettings {
    pub enabled: bool,
    pub auto_frame: bool,
    pub speed_deg_per_sec: f32,
    pub elevation_deg: f32,
    pub distance_factor: f32,
}

impl Default for TurntableSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_frame: true,
            speed_deg_per_sec: 15.0,
            elevation_deg: 30.0,
            distance_factor: 1.35,
        }
    }
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self {
            show_points: false,
            show_point_numbers: false,
            show_vertex_numbers: false,
            show_vertex_normals: true,
            show_primitive_numbers: false,
            show_primitive_normals: true,
            normal_length: 0.2,
            normal_color: Color::srgb(1.0, 1.0, 0.0),
            voxel_grid_line_px: 0.5,
            point_group_viz: None,
            edge_group_viz: None,
            vertex_group_viz: None,
            highlight_active_group: true,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy, Default)]
pub enum ViewportLightingMode {
    #[default]
    HeadlightOnly,
    FullLighting,
    FullLightingWithShadow,
}

#[derive(Resource, Debug)]
pub struct DisplayOptions {
    pub grid: GridSettings,
    pub overlays: OverlaySettings,
    pub turntable: TurntableSettings,
    pub final_geometry_display_mode: DisplayMode,
    pub is_options_collapsed: bool,
    pub is_handle_controls_collapsed: bool,
    pub camera_speed: f32,
    pub ssaa_factor: f32,
    pub view_mode: ViewportViewMode,
    pub uv_pure_mode: bool,
    pub wireframe_ghost_mode: bool,
    #[cfg(feature = "virtual_geometry_meshlet")]
    pub meshlet_virtual_geometry: bool,
    pub camera_rotation: Quat,
    pub lighting_mode: ViewportLightingMode,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            grid: GridSettings {
                show: true,
                show_labels: true,
                major_target_px: 100.0,
            },
            overlays: OverlaySettings::default(),
            turntable: TurntableSettings::default(),
            final_geometry_display_mode: DisplayMode::ShadedAndWireframe,
            is_options_collapsed: false,
            is_handle_controls_collapsed: true,
            camera_speed: 5.0,
            ssaa_factor: 2.0,
            view_mode: ViewportViewMode::Perspective,
            uv_pure_mode: true,
            wireframe_ghost_mode: false,
            #[cfg(feature = "virtual_geometry_meshlet")]
            meshlet_virtual_geometry: false,
            camera_rotation: Quat::IDENTITY,
            lighting_mode: ViewportLightingMode::HeadlightOnly,
        }
    }
}

