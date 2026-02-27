pub mod lighting;
pub mod final_material;
pub mod grid_plane;
pub mod group_visualization;
pub mod normal;
pub mod overlay_visibility;
pub mod point;
pub mod primitive_number;
pub mod sdf_surface;
pub mod uv_material;
pub mod viewport_draw;
pub use cunning_voxel_faces as voxel_faces;
pub mod voxel_faces_desktop_sync;
pub mod wireframe;

// SDF rendering has been moved to egui-wgpu (cunning_ui)
// pub mod sdf_rect;
// pub mod sdf_curve;
