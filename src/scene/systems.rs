//! Scene management systems for 3D viewport rendering.

use bevy::prelude::*;
use bevy::render::sync_world::SyncToRenderWorld;

use crate::{
    nodes::{NodeGraph, NodeGraphResource, parameter::ParameterValue},
    render::{
        final_material::{BackfaceTintExt, FinalMaterial},
        normal::{NormalColor, NormalMarker},
        point::PointMarker,
        wireframe::{WireframeMarker},
        group_visualization::GroupVisualization,
    },
    render::wireframe::WireframeTopology,
    tabs_system::node_editor::cda::navigation::graph_snapshot_by_path,
    ui::UiState,
    ui::NodeEditorState,
    viewport_options::{DisplayOptions, ViewportViewMode, DisplayMode},
    mesh::GeoPrimitive,
    render::uv_material::GlobalUvMaterial,
    render::uv_material::UvMaterial,
    libs::geometry::attrs,
};

use super::geo_material::{
    build_mat_from_matlib, build_subgeo_by_prim_i32, build_subgeo_by_prim_mat, geo_detail_f,
    geo_detail_s, geo_detail_v3, geo_detail_v4,
};

use super::{
    components::{
        DisplayedGeometryInfo, FinalMeshTag, FinalMaterialKey, FinalWireframeTag,
        HighlightPointTag, HighlightPrimitiveTag, TemplateMeshTag, VolumeVizTag,
        VertexNormalTag, PrimitiveNormalTag, DisplayPointTag,
    },
    resources::{UpdateSceneFromGraphParam, OriginalMaterialHandle},
    GraphChanged,
};

use bevy::camera::ScalingMode;

/// The main system that updates the 3D scene based on changes to the node graph.
///
/// This system handles:
/// - Incremental mesh updates when geometry changes (zero-copy where possible)
/// - Material assignment and updates from material library
/// - Wireframe topology generation
/// - Point cloud, vertex normals, and primitive normals visualization
/// - Volume (VDB) rendering with multiple modes
/// - Group highlighting for active group visualization
///
/// NOTE: This is a complex system (~1000 lines) that was originally in main.rs.
/// The full implementation should be migrated here from the original location.
pub fn update_3d_scene_from_node_graph(mut p: UpdateSceneFromGraphParam) {
    puffin::profile_function!();
    let UpdateSceneFromGraphParam {
        mut commands,
        mut graph_changed_reader,
        mut geometry_changed_reader,
        node_graph_res,
        node_editor_state,
        ui_state,
        mut meshes,
        mut materials,
        mut final_materials,
        asset_server,
        mut wireframe_topologies,
        mut query_final_mesh,
        query_final_mat,
        query_has_viz,
        query_final_wireframe_markers,
        query_final_wireframe_entities,
        query_final_wireframe_meshes,
        query_any_mesh3d,
        query_primitive_normals,
        query_volume_viz,
        query_template_mesh,
        mut display_options,
    } = p;

    // Check both legacy GraphChanged and new GeometryChanged events.
    // During migration, both can trigger scene updates. Eventually only GeometryChanged will be used.
    let has_graph_changed = !graph_changed_reader.is_empty();
    let has_geometry_changed = !geometry_changed_reader.is_empty();
    
    if !has_graph_changed && !has_geometry_changed {
        puffin::profile_scope!("early_exit_no_events");
        return;
    }
    graph_changed_reader.clear();
    geometry_changed_reader.clear();

    // Fast-path: UI can spam GraphChanged (dragging, selection, etc). If the final displayed geometry
    // and display options didn't actually change, skip all scene work to avoid viewport hitches.
    let display_options_changed = display_options.is_changed();
    
    let node_graph = graph_snapshot_by_path(&node_graph_res.0, &node_editor_state.cda_path);

    if let Some(display_id) = node_graph.display_node {
        if !display_options_changed {
            if let Some((_entity, _mesh_handle, info)) = query_final_mesh.iter().next() {
                if info.dirty_id == node_graph.final_geometry.dirty_id
                    && node_graph.geometry_cache.contains_key(&display_id)
                {
                    return;
                }
            }
        }
    }

    // Slow path: scene needs update -> clean up transient viz before applying new state.
    let mut meshes_to_remove: Vec<bevy::asset::AssetId<Mesh>> = Vec::new();
    for e in query_volume_viz.iter() {
        if let Ok(m) = query_any_mesh3d.get(e) { meshes_to_remove.push(m.id()); }
        commands.entity(e).despawn();
    }
    for id in meshes_to_remove.drain(..) { let _ = meshes.remove(id); }
    if display_options.overlays.highlight_active_group {
        let (mut point_group, mut edge_group, mut vertex_group) = (None, None, None);
        let candidate = ui_state
            .last_selected_node_id
            .filter(|id| node_graph.nodes.contains_key(id))
            .or(node_graph.display_node);
        if let Some(node_id) = candidate {
            if let Some(node) = node_graph.nodes.get(&node_id) {
                let find_param = |name: &str| -> Option<&ParameterValue> {
                    node.parameters.iter().find(|p| p.name == name).map(|p| &p.value)
                };
                if let Some(ParameterValue::String(name)) = find_param("group_name") {
                    if !name.is_empty() {
                        if let Some(ParameterValue::Int(type_idx)) = find_param("group_type") {
                            match type_idx {
                                0 => point_group = Some(name.clone()),
                                2 => vertex_group = Some(name.clone()),
                                3 => edge_group = Some(name.clone()),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        if display_options.overlays.point_group_viz != point_group { display_options.overlays.point_group_viz = point_group; }
        if display_options.overlays.edge_group_viz != edge_group { display_options.overlays.edge_group_viz = edge_group; }
        if display_options.overlays.vertex_group_viz != vertex_group { display_options.overlays.vertex_group_viz = vertex_group; }
    }
    for entity in query_template_mesh.iter() { commands.entity(entity).despawn(); }
    let display_node_active = node_graph.display_node.is_some();
    if !display_node_active {
        let mut meshes_to_remove: Vec<bevy::asset::AssetId<Mesh>> = Vec::new();
        let mut topologies_to_remove: Vec<Handle<WireframeTopology>> = Vec::new();
        if let Ok((entity, mesh_handle, _)) = query_final_mesh.single() {
            meshes_to_remove.push(mesh_handle.id());
            commands.entity(entity).despawn();
        }
        for m in query_final_wireframe_markers.iter() { topologies_to_remove.push(m.topology.clone()); }
        for m in query_final_wireframe_meshes.iter() { meshes_to_remove.push(m.id()); }
        for entity in query_final_wireframe_entities.iter() { commands.entity(entity).despawn(); }
        for m in query_primitive_normals.iter() { meshes_to_remove.push(m.id()); }
        for h in topologies_to_remove.drain(..) { let _ = wireframe_topologies.remove(h.id()); }
        for id in meshes_to_remove.drain(..) { let _ = meshes.remove(id); }
        return;
    }
    let final_geo = &*node_graph.final_geometry;
    if let Ok((entity, mesh_handle, mut info)) = query_final_mesh.single_mut() {
        if query_has_viz.get(entity).is_err() {
            commands.entity(entity).insert(GroupVisualization::default());
        }
        if info.dirty_id == final_geo.dirty_id { return; }
        if let Some(mesh) = meshes.get_mut(mesh_handle.id()) {
            final_geo.update_bevy_mesh(mesh);
            info.dirty_id = final_geo.dirty_id;
            if let Ok((mat_h, orig_h)) = query_final_mat.get(entity) {
                let handle = mat_h.map(|h| h.0.clone()).or_else(|| orig_h.map(|h| h.0.clone()));
                if let Some(h) = handle {
                    if let Some(dst) = final_materials.get_mut(&h) {
                        let kind = geo_detail_s(final_geo, attrs::MAT_KIND);
                        if kind.is_some() {
                            let mut base = StandardMaterial {
                                base_color: Color::WHITE,
                                cull_mode: None,
                                double_sided: true,
                                ..default()
                            };
                            let tint = geo_detail_v4(final_geo, attrs::MAT_BASECOLOR_TINT).unwrap_or(Vec4::ONE);
                            base.base_color = Color::srgba(tint.x, tint.y, tint.z, tint.w);
                            base.metallic = geo_detail_f(final_geo, attrs::MAT_METALLIC).unwrap_or(0.0).clamp(0.0, 1.0);
                            base.perceptual_roughness = geo_detail_f(final_geo, attrs::MAT_ROUGHNESS).unwrap_or(1.0).clamp(0.0, 1.0);
                            let em = geo_detail_v3(final_geo, attrs::MAT_EMISSIVE).unwrap_or(Vec3::ZERO);
                            base.emissive = bevy::color::LinearRgba::new(em.x, em.y, em.z, 1.0);
                            if let Some(p) = geo_detail_s(final_geo, attrs::MAT_BASECOLOR_TEX).filter(|s| !s.trim().is_empty()) { base.base_color_texture = Some(asset_server.load(p)); }
                            if let Some(p) = geo_detail_s(final_geo, attrs::MAT_NORMAL_TEX).filter(|s| !s.trim().is_empty()) { base.normal_map_texture = Some(asset_server.load(p)); }
                            if let Some(p) = geo_detail_s(final_geo, attrs::MAT_EMISSIVE_TEX).filter(|s| !s.trim().is_empty()) { base.emissive_texture = Some(asset_server.load(p)); }
                            if let Some(p) = geo_detail_s(final_geo, attrs::MAT_ORM_TEX).filter(|s| !s.trim().is_empty()) {
                                let hh: Handle<Image> = asset_server.load(p);
                                base.metallic_roughness_texture = Some(hh.clone());
                                base.occlusion_texture = Some(hh);
                            }
                            dst.base = base;
                        }
                        let prim_n = final_geo.primitives().len();
                        let is_pure_voxel = final_geo
                            .get_detail_attribute("__voxel_pure")
                            .and_then(|a| a.as_slice::<f32>())
                            .and_then(|v| v.first().copied())
                            .map(|v| v > 0.5)
                            .unwrap_or(false)
                            || final_geo
                                .get_primitive_attribute("__voxel_src")
                                .and_then(|a| a.as_slice::<bool>())
                                .map(|m| prim_n > 0 && m.len() == prim_n && m.iter().all(|v| *v))
                                .unwrap_or(false);
                        if is_pure_voxel {
                            let vs = final_geo
                                .get_detail_attribute("__voxel_size")
                                .and_then(|a| a.as_slice::<f32>())
                                .and_then(|v| v.first().copied())
                                .unwrap_or(0.1)
                                .max(0.001);
                            dst.extension.voxel_grid_params = Vec4::new(vs, display_options.overlays.voxel_grid_line_px, 1.0, 0.0);
                            dst.extension.voxel_grid_color = Vec4::new(0.0, 0.0, 0.0, 0.55);
                        } else {
                            dst.extension.voxel_grid_params.z = 0.0;
                        }
                    }
                }
            }
            if let Ok(marker) = query_final_wireframe_markers.single() {
                if let Some(topology) = wireframe_topologies.get_mut(&marker.topology) {
                    let prim_n = final_geo.primitives().len();
                    let is_pure_voxel = final_geo
                        .get_detail_attribute("__voxel_pure")
                        .and_then(|a| a.as_slice::<f32>())
                        .and_then(|v| v.first().copied())
                        .map(|v| v > 0.5)
                        .unwrap_or(false)
                        || final_geo
                            .get_primitive_attribute("__voxel_src")
                            .and_then(|a| a.as_slice::<bool>())
                            .map(|m| prim_n > 0 && m.len() == prim_n && m.iter().all(|v| *v))
                            .unwrap_or(false);
                    let has_polylines = final_geo
                        .primitives()
                        .values()
                        .iter()
                        .any(|p| matches!(p, GeoPrimitive::Polyline(_)));
                    topology.indices = if is_pure_voxel {
                        Vec::new()
                    } else if has_polylines {
                        final_geo.compute_polyline_indices()
                    } else {
                        final_geo.compute_wireframe_indices()
                    };
                }
            }
            let (mut centers, mut normals) = (Vec::new(), Vec::new());
            if let (Some(positions), Some(prim_normals)) = (
                node_graph.final_geometry.get_point_attribute("@P").and_then(|a: &crate::mesh::Attribute| a.as_slice::<Vec3>()),
                node_graph.final_geometry.get_primitive_attribute("@N").and_then(|a: &crate::mesh::Attribute| a.as_slice::<Vec3>()),
            ) {
                for (prim_idx, primitive) in node_graph.final_geometry.primitives().values().iter().enumerate() {
                    let vertices = primitive.vertices();
                    let sum_pos = vertices.iter().fold(Vec3::ZERO, |acc, &v_idx| {
                        node_graph
                            .final_geometry
                            .vertices()
                            .get(v_idx.into())
                            .and_then(|v| node_graph.final_geometry.points().get_dense_index(v.point_id.into()))
                            .and_then(|idx: usize| positions.get(idx))
                            .copied()
                            .unwrap_or(Vec3::ZERO)
                            + acc
                    });
                    let count = vertices.len() as f32;
                    if count > 0.0 {
                        centers.push(sum_pos / count);
                        normals.push(prim_normals.get(prim_idx).copied().unwrap_or(Vec3::Y).normalize_or_zero());
                    }
                }
            }
            if !centers.is_empty() {
                if let Ok(mesh3d) = query_primitive_normals.single() {
                    if let Some(pm) = meshes.get_mut(mesh3d.id()) {
                        pm.insert_attribute(Mesh::ATTRIBUTE_POSITION, centers);
                        pm.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
                    }
                } else {
                    let primitive_normals_vis = if display_options.overlays.show_primitive_normals { Visibility::Visible } else { Visibility::Hidden };
                    let mut prim_mesh = Mesh::new(bevy::render::render_resource::PrimitiveTopology::PointList, bevy::asset::RenderAssetUsages::default());
                    prim_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, centers);
                    prim_mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
                    let prim_mesh_handle = meshes.add(prim_mesh);
                    commands.spawn((
                        Name::new("FinalPrimitiveNormals"),
                        FinalWireframeTag,
                        NormalMarker,
                        PrimitiveNormalTag,
                        NormalColor(Color::srgb(0.6, 0.1, 0.4)),
                        bevy::camera::visibility::NoFrustumCulling,
                        SyncToRenderWorld,
                        Mesh3d(prim_mesh_handle),
                        Transform::default(),
                        primitive_normals_vis,
                    ));
                }
            }
        }
        if !node_graph.final_geometry.volumes.is_empty() {
            let _ = meshes.remove(mesh_handle.id());
            commands.entity(entity).despawn();
        }
        return;
    } else {
        let mut meshes_to_remove: Vec<bevy::asset::AssetId<Mesh>> = Vec::new();
        let mut topologies_to_remove: Vec<Handle<WireframeTopology>> = Vec::new();
        for m in query_final_wireframe_markers.iter() { topologies_to_remove.push(m.topology.clone()); }
        for m in query_final_wireframe_meshes.iter() { meshes_to_remove.push(m.id()); }
        for m in query_primitive_normals.iter() { meshes_to_remove.push(m.id()); }
        for entity in query_final_wireframe_entities.iter() { commands.entity(entity).despawn(); }
        for (e, mh, _) in query_final_mesh.iter() {
            meshes_to_remove.push(mh.id());
            commands.entity(e).despawn();
        }
        for h in topologies_to_remove.drain(..) { let _ = wireframe_topologies.remove(h.id()); }
        for id in meshes_to_remove.drain(..) { let _ = meshes.remove(id); }
    }
    if final_geo.get_point_count() == 0 && final_geo.volumes.is_empty() {
        for (entity, _, _) in query_final_mesh.iter() { commands.entity(entity).despawn(); }
        for entity in query_final_wireframe_entities.iter() { commands.entity(entity).despawn(); }
    } else if final_geo.get_point_count() > 0 {
        let bevy_mesh = meshes.add(final_geo.to_bevy_mesh());
        let has_polylines = final_geo.primitives().values().iter().any(|p| matches!(p, GeoPrimitive::Polyline(_)));
        let wireframe_indices = if has_polylines { final_geo.compute_polyline_indices() } else { final_geo.compute_wireframe_indices() };
        let topology = wireframe_topologies.add(WireframeTopology::new(wireframe_indices));
        let mat_by = geo_detail_s(final_geo, attrs::MAT_BY).unwrap_or_else(|| attrs::SHOP_MATERIALPATH.to_string());
        let mat_attr = if mat_by.starts_with('@') { mat_by } else { format!("@{}", mat_by.trim()) };
        let prim_str = final_geo.get_primitive_attribute(mat_attr.as_str()).and_then(|a: &crate::mesh::Attribute| a.as_slice::<String>());
        let prim_i32 = final_geo.get_primitive_attribute(mat_attr.as_str()).and_then(|a: &crate::mesh::Attribute| a.as_slice::<i32>());
        let (mut keys_s, mut keys_i, mut has_empty) = (Vec::new(), Vec::new(), false);
        if let Some(pm) = prim_str {
            use std::collections::BTreeSet;
            let mut s: BTreeSet<String> = BTreeSet::new();
            for (di, prim) in final_geo.primitives().values().iter().enumerate() {
                if !matches!(prim, GeoPrimitive::Polygon(_)) { continue; }
                let k = pm.get(di).map(|x| x.as_str()).unwrap_or("");
                if k.trim().is_empty() { has_empty = true; continue; }
                s.insert(k.to_string());
            }
            keys_s = s.into_iter().collect();
        } else if let Some(pi) = prim_i32 {
            use std::collections::BTreeSet;
            let mut s: BTreeSet<i32> = BTreeSet::new();
            for (di, prim) in final_geo.primitives().values().iter().enumerate() {
                if !matches!(prim, GeoPrimitive::Polygon(_)) { continue; }
                if let Some(v) = pi.get(di) { s.insert(*v); }
            }
            keys_i = s.into_iter().collect();
        }
        let use_split = !keys_s.is_empty() || !keys_i.is_empty();
        if !keys_s.is_empty() && has_empty { keys_s.insert(0, String::new()); }
        let prim_n = final_geo.primitives().len();
        let is_pure_voxel_final = final_geo
            .get_detail_attribute("__voxel_pure")
            .and_then(|a| a.as_slice::<f32>())
            .and_then(|v| v.first().copied())
            .map(|v| v > 0.5)
            .unwrap_or(false)
            || final_geo
                .get_primitive_attribute("__voxel_src")
                .and_then(|a| a.as_slice::<bool>())
                .map(|m| prim_n > 0 && m.len() == prim_n && m.iter().all(|v| *v))
                .unwrap_or(false);
        let mesh_visibility = if has_polylines { Visibility::Hidden } else if is_pure_voxel_final {
            Visibility::Visible
        } else if matches!(display_options.final_geometry_display_mode, DisplayMode::Shaded | DisplayMode::ShadedAndWireframe) {
            Visibility::Visible
        } else { Visibility::Hidden };
        let wireframe_visibility = if has_polylines { Visibility::Visible } else if is_pure_voxel_final {
            Visibility::Hidden
        } else if matches!(display_options.final_geometry_display_mode, DisplayMode::Wireframe | DisplayMode::ShadedAndWireframe) {
            Visibility::Visible
        } else { Visibility::Hidden };
        if !use_split {
            let Some(base) = build_mat_from_matlib(final_geo, &asset_server, "") else { return; };
            let final_mat = {
                let mut ext = BackfaceTintExt::default();
                let prim_n = final_geo.primitives().len();
                let is_pure_voxel = final_geo
                    .get_detail_attribute("__voxel_pure")
                    .and_then(|a| a.as_slice::<f32>())
                    .and_then(|v| v.first().copied())
                    .map(|v| v > 0.5)
                    .unwrap_or(false)
                    || final_geo
                        .get_primitive_attribute("__voxel_src")
                        .and_then(|a| a.as_slice::<bool>())
                        .map(|m| prim_n > 0 && m.len() == prim_n && m.iter().all(|v| *v))
                        .unwrap_or(false);
                if is_pure_voxel {
                    let vs = final_geo
                        .get_detail_attribute("__voxel_size")
                        .and_then(|a| a.as_slice::<f32>())
                        .and_then(|v| v.first().copied())
                        .unwrap_or(0.1)
                        .max(0.001);
                    ext.voxel_grid_params = Vec4::new(vs, display_options.overlays.voxel_grid_line_px, 1.0, 0.0);
                    ext.voxel_grid_color = Vec4::new(0.0, 0.0, 0.0, 0.55);
                }
                final_materials.add(FinalMaterial { base, extension: ext })
            };
            commands.spawn((
                Name::new("FinalMesh"),
                FinalMeshTag,
                DisplayedGeometryInfo { dirty_id: final_geo.dirty_id },
                Mesh3d(bevy_mesh.clone()),
                MeshMaterial3d(final_mat),
                Transform::default(),
                mesh_visibility,
                GroupVisualization::default(),
            ));
        } else if !keys_s.is_empty() {
            for key in keys_s.iter() {
                let Some(sub_geo) = build_subgeo_by_prim_mat(final_geo, &mat_attr, key) else { continue; };
                let sub_mesh = meshes.add(sub_geo.to_bevy_mesh());
                let Some(base) = build_mat_from_matlib(final_geo, &asset_server, key) else { continue; };
                let h = {
                    let mut ext = BackfaceTintExt::default();
                    let prim_n = sub_geo.primitives().len();
                    let is_pure_voxel = sub_geo
                        .get_primitive_attribute("__voxel_src")
                        .and_then(|a| a.as_slice::<bool>())
                        .map(|m| prim_n > 0 && m.len() == prim_n && m.iter().all(|v| *v))
                        .unwrap_or(false);
                    if is_pure_voxel {
                        let vs = sub_geo
                            .get_detail_attribute("__voxel_size")
                            .and_then(|a| a.as_slice::<f32>())
                            .and_then(|v| v.first().copied())
                            .unwrap_or(0.1)
                            .max(0.001);
                        ext.voxel_grid_params = Vec4::new(vs, display_options.overlays.voxel_grid_line_px, 1.0, 0.0);
                        ext.voxel_grid_color = Vec4::new(0.0, 0.0, 0.0, 0.55);
                    }
                    final_materials.add(FinalMaterial { base, extension: ext })
                };
                let label = if key.is_empty() { "default".to_string() } else { key.clone() };
                commands.spawn((
                    Name::new(format!("FinalMesh::{label}")),
                    FinalMeshTag,
                    FinalMaterialKey(key.clone()),
                    DisplayedGeometryInfo { dirty_id: final_geo.dirty_id },
                    Mesh3d(sub_mesh.clone()),
                    MeshMaterial3d(h),
                    Transform::default(),
                    mesh_visibility,
                    GroupVisualization::default(),
                ));
            }
        } else {
            for key in keys_i.iter() {
                let Some(sub_geo) = build_subgeo_by_prim_i32(final_geo, &mat_attr, *key) else { continue; };
                let sub_mesh = meshes.add(sub_geo.to_bevy_mesh());
                let Some(base) = build_mat_from_matlib(final_geo, &asset_server, &key.to_string()) else { continue; };
                let h = {
                    let mut ext = BackfaceTintExt::default();
                    let prim_n = sub_geo.primitives().len();
                    let is_pure_voxel = sub_geo
                        .get_primitive_attribute("__voxel_src")
                        .and_then(|a| a.as_slice::<bool>())
                        .map(|m| prim_n > 0 && m.len() == prim_n && m.iter().all(|v| *v))
                        .unwrap_or(false);
                    if is_pure_voxel {
                        let vs = sub_geo
                            .get_detail_attribute("__voxel_size")
                            .and_then(|a| a.as_slice::<f32>())
                            .and_then(|v| v.first().copied())
                            .unwrap_or(0.1)
                            .max(0.001);
                        ext.voxel_grid_params = Vec4::new(vs, display_options.overlays.voxel_grid_line_px, 1.0, 0.0);
                        ext.voxel_grid_color = Vec4::new(0.0, 0.0, 0.0, 0.55);
                    }
                    final_materials.add(FinalMaterial { base, extension: ext })
                };
                commands.spawn((
                    Name::new(format!("FinalMesh::class_{key}")),
                    FinalMeshTag,
                    FinalMaterialKey(key.to_string()),
                    DisplayedGeometryInfo { dirty_id: final_geo.dirty_id },
                    Mesh3d(sub_mesh.clone()),
                    MeshMaterial3d(h),
                    Transform::default(),
                    mesh_visibility,
                    GroupVisualization::default(),
                ));
            }
        }
        commands.spawn((
            Name::new("FinalWireframe"),
            FinalWireframeTag,
            WireframeMarker { topology },
            Mesh3d(bevy_mesh.clone()),
            Transform::default(),
            wireframe_visibility,
        ));
        let points_visibility = if display_options.overlays.show_points { Visibility::Visible } else { Visibility::Hidden };
        commands.spawn((
            Name::new("FinalPoints"),
            FinalWireframeTag,
            PointMarker,
            SyncToRenderWorld,
            Mesh3d(bevy_mesh.clone()),
            Transform::default(),
            points_visibility,
        ));
        let vertex_normals_vis = if display_options.overlays.show_vertex_normals { Visibility::Visible } else { Visibility::Hidden };
        commands.spawn((
            Name::new("FinalNormals"),
            FinalWireframeTag,
            NormalMarker,
            VertexNormalTag,
            NormalColor(Color::srgb(0.5, 0.6, 0.2)),
            bevy::camera::visibility::NoFrustumCulling,
            SyncToRenderWorld,
            Mesh3d(bevy_mesh.clone()),
            Transform::default(),
            vertex_normals_vis,
        ));
        let primitive_normals_vis = if display_options.overlays.show_primitive_normals { Visibility::Visible } else { Visibility::Hidden };
        let (mut centers, mut normals) = (Vec::new(), Vec::new());
        if let (Some(positions), Some(prim_normals)) = (
            node_graph.final_geometry.get_point_attribute("@P").and_then(|a: &crate::mesh::Attribute| a.as_slice::<Vec3>()),
            node_graph.final_geometry.get_primitive_attribute("@N").and_then(|a: &crate::mesh::Attribute| a.as_slice::<Vec3>()),
        ) {
            for (prim_idx, primitive) in node_graph.final_geometry.primitives().values().iter().enumerate() {
                let vertices = primitive.vertices();
                let sum_pos = vertices.iter().fold(Vec3::ZERO, |acc, &v_idx| {
                    node_graph
                        .final_geometry
                        .vertices()
                        .get(v_idx.into())
                        .and_then(|v| node_graph.final_geometry.points().get_dense_index(v.point_id.into()))
                        .and_then(|idx: usize| positions.get(idx))
                        .copied()
                        .unwrap_or(Vec3::ZERO)
                        + acc
                });
                let count = vertices.len() as f32;
                if count > 0.0 {
                    centers.push(sum_pos / count);
                    normals.push(prim_normals.get(prim_idx).copied().unwrap_or(Vec3::Y).normalize_or_zero());
                }
            }
        }
        if !centers.is_empty() {
            let mut prim_mesh = Mesh::new(bevy::render::render_resource::PrimitiveTopology::PointList, bevy::asset::RenderAssetUsages::default());
            prim_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, centers);
            prim_mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
            let prim_mesh_handle = meshes.add(prim_mesh);
            commands.spawn((
                Name::new("FinalPrimitiveNormals"),
                FinalWireframeTag,
                NormalMarker,
                PrimitiveNormalTag,
                NormalColor(Color::srgb(0.6, 0.1, 0.4)),
                bevy::camera::visibility::NoFrustumCulling,
                SyncToRenderWorld,
                Mesh3d(prim_mesh_handle),
                Transform::default(),
                primitive_normals_vis,
            ));
        }
    }
    for volume_handle in &node_graph.final_geometry.volumes {
        if let Ok(grid) = volume_handle.grid.read() {
            let show_points = node_graph
                .final_geometry
                .get_detail_attribute("display_points")
                .and_then(|attr| attr.as_slice::<f32>().and_then(|vals| vals.get(0).map(|&v| v > 0.5)))
                .unwrap_or(false);
            if show_points {
                let local_points = grid.get_active_voxels();
                let points: Vec<Vec3> = local_points.iter().map(|p| volume_handle.transform.transform_point3(*p)).collect();
                if !points.is_empty() {
                    let point_mesh = crate::mesh::create_point_cloud_mesh(&points);
                    commands.spawn((
                        Name::new("VDB Visualization"),
                        VolumeVizTag,
                        DisplayedGeometryInfo { dirty_id: final_geo.dirty_id },
                        Mesh3d(meshes.add(point_mesh)),
                        MeshMaterial3d(materials.add(StandardMaterial { base_color: Color::WHITE, unlit: true, ..default() })),
                        Transform::default(),
                        Visibility::Visible,
                    ));
                }
            } else {
                let mesh_geo = crate::nodes::vdb::vdb_to_mesh::vdb_to_geometry(volume_handle, 0.0, false, false);
                if mesh_geo.get_point_count() > 0 {
                    let bevy_mesh = meshes.add(mesh_geo.to_bevy_mesh());
                    commands.spawn((
                        Name::new("VDB Visualization"),
                        VolumeVizTag,
                        DisplayedGeometryInfo { dirty_id: final_geo.dirty_id },
                        Mesh3d(bevy_mesh),
                        MeshMaterial3d(materials.add(StandardMaterial {
                            base_color: Color::srgb(0.6, 0.6, 0.65),
                            perceptual_roughness: 0.8,
                            metallic: 0.1,
                            ..default()
                        })),
                        Transform::default(),
                        Visibility::Visible,
                    ));
                }
            }
        }
    }
}

/// System that updates the visibility of the final mesh based on display options.
pub fn update_final_mesh_visibility_system(
    display_options: Res<DisplayOptions>,
    mut query_mesh: Query<&mut Visibility, With<FinalMeshTag>>,
    query_mat: Query<&MeshMaterial3d<FinalMaterial>, With<FinalMeshTag>>,
    materials: Res<Assets<FinalMaterial>>,
    mut query_wireframe: Query<
        &mut Visibility,
        (
            With<FinalWireframeTag>,
            With<WireframeMarker>,
            Without<FinalMeshTag>,
        ),
    >,
) {
    let is_pure_uv_mode = display_options.view_mode == ViewportViewMode::UV && display_options.uv_pure_mode;
    let is_node_image_mode = display_options.view_mode == ViewportViewMode::NodeImage;
    let is_pure_voxel = query_mat
        .single()
        .ok()
        .and_then(|h| materials.get(&h.0))
        .map(|m| m.extension.voxel_grid_params.z > 0.5)
        .unwrap_or(false);
    if let Ok(mut visibility) = query_mesh.single_mut() {
        let want = if is_pure_voxel {
            Visibility::Hidden
        } else if is_node_image_mode {
            Visibility::Hidden
        } else if is_pure_uv_mode || matches!(display_options.final_geometry_display_mode, DisplayMode::Shaded | DisplayMode::ShadedAndWireframe) {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *visibility != want { *visibility = want; }
    }
    if let Ok(mut visibility) = query_wireframe.single_mut() {
        let want = if is_pure_voxel {
            Visibility::Hidden
        } else if is_node_image_mode {
            Visibility::Hidden
        } else if is_pure_uv_mode || matches!(display_options.final_geometry_display_mode, DisplayMode::Wireframe | DisplayMode::ShadedAndWireframe) {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *visibility != want { *visibility = want; }
    }
}

/// System that updates the material of the final mesh based on display options.
pub fn update_final_mesh_material_system(
    display_options: Res<DisplayOptions>,
    query: Query<&MeshMaterial3d<FinalMaterial>, With<FinalMeshTag>>,
    mut materials: ResMut<Assets<FinalMaterial>>,
) {
    let is_pure_uv_mode = display_options.view_mode == ViewportViewMode::UV && display_options.uv_pure_mode;
    for material_handle in &query {
        if let Some(material) = materials.get_mut(&material_handle.0) {
            if is_pure_uv_mode {
                material.base.unlit = true;
                material.base.base_color = Color::srgba(3.0, 3.0, 3.0, 0.5);
                material.base.emissive = Color::linear_rgb(3.0, 3.0, 3.0).into();
                material.base.alpha_mode = AlphaMode::Blend;
                material.base.double_sided = true;
                material.base.cull_mode = None;
            }
        }
    }
}

pub fn sync_uv_view_mode(
    display_options: Res<DisplayOptions>,
    mut last_view_mode: Local<ViewportViewMode>,
    mut camera_query: Query<(&mut Transform, &mut Projection), With<crate::MainCamera>>,
    mut query_final_mesh: Query<
        (
            Entity,
            Option<&MeshMaterial3d<FinalMaterial>>,
            Option<&MeshMaterial3d<UvMaterial>>,
        ),
        With<FinalMeshTag>,
    >,
    mut commands: Commands,
    uv_material: Res<GlobalUvMaterial>,
    mut original_materials: Query<&mut OriginalMaterialHandle>,
    mut graph_changed_events: MessageReader<GraphChanged>,
) {
    let mode_changed = display_options.view_mode != *last_view_mode;
    let graph_changed = !graph_changed_events.is_empty();
    if graph_changed { graph_changed_events.clear(); }
    if mode_changed { *last_view_mode = display_options.view_mode; }
    if !mode_changed && graph_changed && display_options.view_mode != ViewportViewMode::UV { return; }
    if let Ok((mut transform, mut projection)) = camera_query.single_mut() {
        if matches!(display_options.view_mode, ViewportViewMode::UV | ViewportViewMode::NodeImage) {
            let mut o = OrthographicProjection::default_3d();
            o.scaling_mode = ScalingMode::FixedVertical { viewport_height: 1.6 };
            o.viewport_origin = Vec2::new(0.5, 0.5);
            *projection = Projection::Orthographic(o);
            *transform = Transform::from_xyz(0.5, 0.5, 5.0).looking_at(Vec3::new(0.5, 0.5, 0.0), Vec3::Y);
        } else if mode_changed && matches!(display_options.view_mode, ViewportViewMode::Perspective) {
            *projection = Projection::Perspective(PerspectiveProjection::default());
            *transform = Transform::from_xyz(-2.0, 2.5, 5.0).looking_at(Vec3::ZERO, Vec3::Y);
        }
    }
    for (entity, final_handle, uv_handle) in query_final_mesh.iter_mut() {
        if display_options.view_mode == ViewportViewMode::UV {
            if final_handle.is_some() {
                if let Some(h) = final_handle { commands.entity(entity).insert(OriginalMaterialHandle(h.0.clone())); }
                commands.entity(entity).remove::<MeshMaterial3d<FinalMaterial>>().insert(MeshMaterial3d(uv_material.0.clone()));
            } else if uv_handle.is_none() {
                commands.entity(entity).insert(MeshMaterial3d(uv_material.0.clone()));
            }
        } else if uv_handle.is_some() {
            if let Ok(orig) = original_materials.get(entity) {
                commands.entity(entity).remove::<MeshMaterial3d<UvMaterial>>().insert(MeshMaterial3d(orig.0.clone())).remove::<OriginalMaterialHandle>();
            } else {
                warn!("Lost OriginalMaterialHandle when switching back from UV mode!");
            }
        }
    }
}
