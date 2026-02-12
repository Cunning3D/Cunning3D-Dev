//! Desktop: drive voxel_faces root from final geometry.
use bevy::prelude::*;
use uuid::Uuid;

pub fn sync_voxel_faces_root_from_final_geo_system(
    node_graph_res: Res<crate::nodes::NodeGraphResource>,
    ui_state: Res<crate::ui::UiState>,
    mut root: ResMut<cunning_voxel_faces::VoxelPreviewRoot>,
) {
    let g = &node_graph_res.0;
    // Primary path: show voxel preview when the display geometry is tagged as pure voxel.
    let final_geo = &*g.final_geometry;
    let is_pure = final_geo
        .get_detail_attribute("__voxel_pure")
        .and_then(|a| a.as_slice::<f32>())
        .and_then(|v| v.first().copied())
        .unwrap_or(0.0)
        > 0.5;
    let node_id = final_geo
        .get_detail_attribute("__voxel_node")
        .and_then(|a| a.as_slice::<String>())
        .and_then(|v| v.first().cloned())
        .and_then(|s| Uuid::parse_str(&s).ok());
    let voxel_size = final_geo
        .get_detail_attribute("__voxel_size")
        .and_then(|a| a.as_slice::<f32>())
        .and_then(|v| v.first().copied())
        .unwrap_or(0.0)
        .max(0.001);

    if is_pure && node_id.is_some() {
        root.node_id = node_id;
        root.voxel_size = voxel_size;
        return;
    }

    // Secondary path for editor interaction: drive voxel preview from the active voxel-edit target (selection).
    // IMPORTANT: do not overwrite the cook cache here; the VoxelEdit cook cache includes upstream base voxels (e.g. File->VOX).
    if let Some(target) = crate::coverlay_bevy_ui::resolve_voxel_edit_target(&ui_state, g) {
        let vs = crate::coverlay_bevy_ui::voxel_size_for_target(g, target).max(0.001);
        let cmds = crate::coverlay_bevy_ui::read_voxel_cmds(g, target);
        let keyed = match target {
            crate::coverlay_bevy_ui::VoxelEditTarget::Direct(node_id) => node_id,
            crate::coverlay_bevy_ui::VoxelEditTarget::Cda { inst_id, internal_id } => {
                cunning_kernel::nodes::voxel::voxel_edit::voxel_render_key_for_instance(inst_id, internal_id)
            }
        };
        // Fallback: if the cook cache is not yet initialized, seed it from cmdlist only (empty base).
        if cunning_kernel::nodes::voxel::voxel_edit::voxel_render_chunks_gen(keyed) == 0 {
            cunning_kernel::nodes::voxel::voxel_edit::voxel_render_sync_cmds(keyed, vs, &cmds);
        }
        root.node_id = Some(keyed);
        root.voxel_size = vs;
        return;
    }

    root.node_id = None;
    root.voxel_size = 0.0;
}

