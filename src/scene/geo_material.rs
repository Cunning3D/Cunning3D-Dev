use bevy::prelude::*;

#[inline]
pub(crate) fn geo_detail_s(geo: &crate::mesh::Geometry, name: &str) -> Option<String> {
    geo.get_detail_attribute(name)
        .and_then(|a| a.as_slice::<String>())
        .and_then(|v| v.get(0))
        .cloned()
}

#[inline]
pub(crate) fn geo_detail_f(geo: &crate::mesh::Geometry, name: &str) -> Option<f32> {
    geo.get_detail_attribute(name)
        .and_then(|a| a.as_slice::<f32>())
        .and_then(|v| v.get(0))
        .copied()
}

#[inline]
pub(crate) fn geo_detail_v3(geo: &crate::mesh::Geometry, name: &str) -> Option<Vec3> {
    geo.get_detail_attribute(name)
        .and_then(|a| a.as_slice::<Vec3>())
        .and_then(|v| v.get(0))
        .copied()
}

#[inline]
pub(crate) fn geo_detail_v4(geo: &crate::mesh::Geometry, name: &str) -> Option<Vec4> {
    geo.get_detail_attribute(name)
        .and_then(|a| a.as_slice::<Vec4>())
        .and_then(|v| v.get(0))
        .copied()
}

pub(crate) fn build_mat_from_matlib(
    geo: &crate::mesh::Geometry,
    asset_server: &AssetServer,
    key: &str,
) -> Option<StandardMaterial> {
    let mut m = StandardMaterial {
        base_color: Color::WHITE,
        cull_mode: None,
        double_sided: true,
        ..default()
    };
    if key.trim().is_empty() {
        let kind = geo_detail_s(geo, crate::libs::geometry::attrs::MAT_KIND);
        if kind.is_none() {
            return Some(m);
        }
        let tint = geo_detail_v4(geo, crate::libs::geometry::attrs::MAT_BASECOLOR_TINT).unwrap_or(Vec4::ONE);
        m.base_color = Color::srgba(tint.x, tint.y, tint.z, tint.w);
        m.metallic = geo_detail_f(geo, crate::libs::geometry::attrs::MAT_METALLIC)
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        m.perceptual_roughness = geo_detail_f(geo, crate::libs::geometry::attrs::MAT_ROUGHNESS)
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        {
            let e = geo_detail_v3(geo, crate::libs::geometry::attrs::MAT_EMISSIVE).unwrap_or(Vec3::ZERO);
            m.emissive = Color::linear_rgb(e.x, e.y, e.z).into();
        }
        if let Some(p) = geo_detail_s(geo, crate::libs::geometry::attrs::MAT_BASECOLOR_TEX).filter(|s| !s.trim().is_empty()) {
            m.base_color_texture = Some(asset_server.load(p));
        }
        if let Some(p) = geo_detail_s(geo, crate::libs::geometry::attrs::MAT_NORMAL_TEX).filter(|s| !s.trim().is_empty()) {
            m.normal_map_texture = Some(asset_server.load(p));
        }
        if let Some(p) = geo_detail_s(geo, crate::libs::geometry::attrs::MAT_EMISSIVE_TEX).filter(|s| !s.trim().is_empty()) {
            m.emissive_texture = Some(asset_server.load(p));
        }
        if let Some(p) = geo_detail_s(geo, crate::libs::geometry::attrs::MAT_ORM_TEX).filter(|s| !s.trim().is_empty()) {
            let h: Handle<Image> = asset_server.load(p);
            m.metallic_roughness_texture = Some(h.clone());
            m.occlusion_texture = Some(h);
        }
        return Some(m);
    }
    let pfx = format!("__cunning.matlib.{key}.");
    let s = |n: &str| geo_detail_s(geo, &format!("{pfx}{n}"));
    let f = |n: &str| geo_detail_f(geo, &format!("{pfx}{n}"));
    let v3 = |n: &str| geo_detail_v3(geo, &format!("{pfx}{n}"));
    let v4 = |n: &str| geo_detail_v4(geo, &format!("{pfx}{n}"));
    let tint = v4("basecolor_tint")?;
    m.base_color = Color::srgba(tint.x, tint.y, tint.z, tint.w);
    m.metallic = f("metallic")?.clamp(0.0, 1.0);
    m.perceptual_roughness = f("roughness")?.clamp(0.0, 1.0);
    {
        let e = v3("emissive")?;
        m.emissive = Color::linear_rgb(e.x, e.y, e.z).into();
    }
    if let Some(p) = s("basecolor_tex").filter(|s| !s.trim().is_empty()) {
        m.base_color_texture = Some(asset_server.load(p));
    }
    if let Some(p) = s("normal_tex").filter(|s| !s.trim().is_empty()) {
        m.normal_map_texture = Some(asset_server.load(p));
    }
    if let Some(p) = s("emissive_tex").filter(|s| !s.trim().is_empty()) {
        m.emissive_texture = Some(asset_server.load(p));
    }
    if let Some(p) = s("orm_tex").filter(|s| !s.trim().is_empty()) {
        let h: Handle<Image> = asset_server.load(p);
        m.metallic_roughness_texture = Some(h.clone());
        m.occlusion_texture = Some(h);
    }
    Some(m)
}

pub(crate) fn build_subgeo_by_prim_mat(
    src: &crate::mesh::Geometry,
    prim_attr: &str,
    key: &str,
) -> Option<crate::mesh::Geometry> {
    use crate::libs::geometry::attrs as ga;
    use crate::mesh::{Attribute as A, GeoPrimitive, PolygonPrim};
    use std::collections::HashMap;
    let pm = src.get_primitive_attribute(prim_attr).and_then(|a| a.as_slice::<String>())?;
    let p_attr = src.get_point_attribute(ga::P).and_then(|a| a.as_slice::<Vec3>())?;
    let n_attr = src.get_vertex_attribute(ga::N).and_then(|a| a.as_slice::<Vec3>());
    let uv_v = src.get_vertex_attribute(ga::UV).and_then(|a| a.as_slice::<Vec2>());
    let uv_p = src.get_point_attribute(ga::UV).and_then(|a| a.as_slice::<Vec2>());
    let mut out = crate::mesh::Geometry::new();
    let mut p_map: HashMap<usize, (crate::libs::geometry::ids::PointId, usize)> = HashMap::new();
    let mut v_map: HashMap<usize, (crate::libs::geometry::ids::VertexId, usize)> = HashMap::new();
    let mut used_any = false;
    for (prim_di, prim) in src.primitives().values().iter().enumerate() {
        if !matches!(prim, GeoPrimitive::Polygon(_)) {
            continue;
        }
        let k = pm.get(prim_di).map(|s| s.as_str()).unwrap_or("");
        if k != key {
            continue;
        }
        used_any = true;
        let GeoPrimitive::Polygon(poly) = prim else { continue };
        let mut new_vs = Vec::with_capacity(poly.vertices.len());
        for &vid in &poly.vertices {
            let vdi = src.vertices().get_dense_index(vid.into())?;
            let (new_vid, _new_vdi) = if let Some(v) = v_map.get(&vdi).copied() {
                v
            } else {
                let v = src.vertices().get(vid.into())?;
                let pdi = src.points().get_dense_index(v.point_id.into())?;
                let (new_pid, _new_pdi) = if let Some(p) = p_map.get(&pdi).copied() {
                    p
                } else {
                    let pid = out.add_point();
                    let di = out.points().len() - 1;
                    p_map.insert(pdi, (pid, di));
                    (pid, di)
                };
                let nid = out.add_vertex(new_pid);
                let ndi = out.vertices().len() - 1;
                v_map.insert(vdi, (nid, ndi));
                (nid, ndi)
            };
            new_vs.push(new_vid);
        }
        out.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: new_vs }));
    }
    if !used_any {
        return None;
    }
    let mut pos = vec![Vec3::ZERO; out.points().len()];
    let mut uvp_out: Option<Vec<Vec2>> = uv_p.map(|_| vec![Vec2::ZERO; out.points().len()]);
    for (old_pdi, (_pid, new_pdi)) in p_map.iter() {
        if let Some(v) = pos.get_mut(*new_pdi) {
            *v = *p_attr.get(*old_pdi)?;
        }
        if let Some(u) = uvp_out.as_mut() {
            if let Some(x) = u.get_mut(*new_pdi) {
                *x = *uv_p?.get(*old_pdi)?;
            }
        }
    }
    out.insert_point_attribute(ga::P, A::new(pos));
    if let Some(u) = uvp_out {
        out.insert_point_attribute(ga::UV, A::new(u));
    }
    if let Some(n) = n_attr {
        let mut vn = vec![Vec3::Y; out.vertices().len()];
        for (old_vdi, (_vid, new_vdi)) in v_map.iter() {
            if let Some(x) = vn.get_mut(*new_vdi) {
                *x = *n.get(*old_vdi)?;
            }
        }
        out.insert_vertex_attribute(ga::N, A::new(vn));
    }
    if let Some(u) = uv_v {
        let mut vu = vec![Vec2::ZERO; out.vertices().len()];
        for (old_vdi, (_vid, new_vdi)) in v_map.iter() {
            if let Some(x) = vu.get_mut(*new_vdi) {
                *x = *u.get(*old_vdi)?;
            }
        }
        out.insert_vertex_attribute(ga::UV, A::new(vu));
    }
    Some(out)
}

pub(crate) fn build_subgeo_by_prim_i32(
    src: &crate::mesh::Geometry,
    prim_attr: &str,
    key: i32,
) -> Option<crate::mesh::Geometry> {
    use crate::libs::geometry::attrs as ga;
    use crate::mesh::{Attribute as A, GeoPrimitive, PolygonPrim};
    use std::collections::HashMap;
    let pm = src.get_primitive_attribute(prim_attr).and_then(|a| a.as_slice::<i32>())?;
    let p_attr = src.get_point_attribute(ga::P).and_then(|a| a.as_slice::<Vec3>())?;
    let n_attr = src.get_vertex_attribute(ga::N).and_then(|a| a.as_slice::<Vec3>());
    let uv_v = src.get_vertex_attribute(ga::UV).and_then(|a| a.as_slice::<Vec2>());
    let uv_p = src.get_point_attribute(ga::UV).and_then(|a| a.as_slice::<Vec2>());
    let mut out = crate::mesh::Geometry::new();
    let mut p_map: HashMap<usize, (crate::libs::geometry::ids::PointId, usize)> = HashMap::new();
    let mut v_map: HashMap<usize, (crate::libs::geometry::ids::VertexId, usize)> = HashMap::new();
    let mut used_any = false;
    for (prim_di, prim) in src.primitives().values().iter().enumerate() {
        if !matches!(prim, GeoPrimitive::Polygon(_)) {
            continue;
        }
        if pm.get(prim_di).copied().unwrap_or_default() != key {
            continue;
        }
        used_any = true;
        let GeoPrimitive::Polygon(poly) = prim else { continue };
        let mut new_vs = Vec::with_capacity(poly.vertices.len());
        for &vid in &poly.vertices {
            let vdi = src.vertices().get_dense_index(vid.into())?;
            let (new_vid, _new_vdi) = if let Some(v) = v_map.get(&vdi).copied() {
                v
            } else {
                let v = src.vertices().get(vid.into())?;
                let pdi = src.points().get_dense_index(v.point_id.into())?;
                let (new_pid, _new_pdi) = if let Some(p) = p_map.get(&pdi).copied() {
                    p
                } else {
                    let pid = out.add_point();
                    let di = out.points().len() - 1;
                    p_map.insert(pdi, (pid, di));
                    (pid, di)
                };
                let nid = out.add_vertex(new_pid);
                let ndi = out.vertices().len() - 1;
                v_map.insert(vdi, (nid, ndi));
                (nid, ndi)
            };
            new_vs.push(new_vid);
        }
        out.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: new_vs }));
    }
    if !used_any {
        return None;
    }
    let mut pos = vec![Vec3::ZERO; out.points().len()];
    let mut uvp_out: Option<Vec<Vec2>> = uv_p.map(|_| vec![Vec2::ZERO; out.points().len()]);
    for (old_pdi, (_pid, new_pdi)) in p_map.iter() {
        if let Some(v) = pos.get_mut(*new_pdi) {
            *v = *p_attr.get(*old_pdi)?;
        }
        if let Some(u) = uvp_out.as_mut() {
            if let Some(x) = u.get_mut(*new_pdi) {
                *x = *uv_p?.get(*old_pdi)?;
            }
        }
    }
    out.insert_point_attribute(ga::P, A::new(pos));
    if let Some(u) = uvp_out {
        out.insert_point_attribute(ga::UV, A::new(u));
    }
    if let Some(n) = n_attr {
        let mut vn = vec![Vec3::Y; out.vertices().len()];
        for (old_vdi, (_vid, new_vdi)) in v_map.iter() {
            if let Some(x) = vn.get_mut(*new_vdi) {
                *x = *n.get(*old_vdi)?;
            }
        }
        out.insert_vertex_attribute(ga::N, A::new(vn));
    }
    if let Some(u) = uv_v {
        let mut vu = vec![Vec2::ZERO; out.vertices().len()];
        for (old_vdi, (_vid, new_vdi)) in v_map.iter() {
            if let Some(x) = vu.get_mut(*new_vdi) {
                *x = *u.get(*old_vdi)?;
            }
        }
        out.insert_vertex_attribute(ga::UV, A::new(vu));
    }
    Some(out)
}

