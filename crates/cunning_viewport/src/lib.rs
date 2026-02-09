use bevy::prelude::*;
use bevy::camera::RenderTarget;
use bevy::window::WindowRef;

pub mod viewport_options;
pub mod icons;
pub mod layout;
pub mod camera_sync;
pub mod nav_input;
pub mod input;
pub mod camera;
pub mod grid;
pub mod hud;
pub mod viewport_ui;
pub mod coverlay_dock;
pub mod voxel_coverlay_ui;

#[derive(Component)]
pub struct MainCamera;

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportUiMode { Standalone, Embedded }
impl Default for ViewportUiMode { fn default() -> Self { Self::Standalone } }

#[inline]
fn viewport_ui_is_standalone(m: Res<ViewportUiMode>) -> bool { matches!(*m, ViewportUiMode::Standalone) }

#[derive(Resource, Clone, Default)]
pub struct ViewportRenderState(pub std::sync::Arc<std::sync::Mutex<ViewportRenderStateInner>>);

#[derive(Default)]
pub struct ViewportRenderStateInner {
    pub image_handle: Handle<Image>,
    pub egui_texture_id: Option<bevy_egui::egui::TextureId>,
    pub viewport_size: bevy_egui::egui::Vec2,
}

pub struct CunningViewportPlugin;

impl Plugin for CunningViewportPlugin {
    fn build(&self, app: &mut App) {
        use crate::viewport_options::{CameraRotateEvent, SetCameraViewEvent};
        app
            .add_message::<SetCameraViewEvent>()
            .add_message::<CameraRotateEvent>()
            .add_message::<bevy::window::RequestRedraw>()
            .init_resource::<viewport_options::DisplayOptions>()
            .init_resource::<layout::ViewportLayout>()
            .init_resource::<nav_input::NavigationInput>()
            .init_resource::<camera::ViewportInteractionState>()
            .init_resource::<viewport_ui::ViewportUiState>()
            .init_resource::<ViewportUiMode>()
            .init_resource::<ViewportRenderState>()
            .add_systems(Startup, setup_viewport_camera_system)
            .add_systems(
                Update,
                (
                    viewport_ui::viewport_ui_system.run_if(viewport_ui_is_standalone),
                    camera_sync::sync_main_camera_viewport.after(viewport_ui::viewport_ui_system),
                    input::input_mapping_system.after(viewport_ui::viewport_ui_system),
                    camera::handle_camera_view_events.after(viewport_ui::viewport_ui_system),
                    camera::handle_camera_rotate_events.after(viewport_ui::viewport_ui_system),
                    camera::camera_transition_system,
                    camera::camera_control_system.after(input::input_mapping_system),
                    grid::update_grid_labels_system.after(viewport_ui::viewport_ui_system),
                ),
            );
    }
}

fn setup_viewport_camera_system(
    mut commands: Commands,
    q: Query<Entity, With<MainCamera>>,
    display_options: Res<viewport_options::DisplayOptions>,
) {
    if q.iter().next().is_some() { return; }
    commands.spawn((
        Camera::default(),
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection::default()),
        RenderTarget::Window(WindowRef::Primary),
        Transform::from_xyz(-2.0, 2.5, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        GlobalTransform::default(),
        Visibility::default(),
        InheritedVisibility::default(),
        ViewVisibility::default(),
        #[cfg(target_arch = "wasm32")] Msaa::Sample4,
        #[cfg(not(target_arch = "wasm32"))] Msaa::Sample4,
        // WebGL2 can't compile some MSAA shader variants (no GL_OES_sample_variables / gl_SampleID).
        // On wasm we keep MSAA Off, and (if the host app enabled FXAA plugin) we enable FXAA per-camera.
        #[cfg(target_arch = "wasm32")]
        bevy_anti_alias::fxaa::Fxaa {
            enabled: true,
            edge_threshold: bevy_anti_alias::fxaa::Sensitivity::Ultra,
            edge_threshold_min: bevy_anti_alias::fxaa::Sensitivity::Ultra,
        },
        MainCamera,
        camera::CameraController { speed: display_options.camera_speed, ..Default::default() },
    ));
    commands.spawn((PointLight { intensity: 2_500_000.0, range: 1000.0, ..default() }, Transform::from_xyz(3.0, 3.0, 3.0)));
}

