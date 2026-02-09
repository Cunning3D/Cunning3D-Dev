//! Height Displace HighPoly V2: displace by UV height map, optionally SDF remesh.

use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::mesh::{Attribute, Geometry};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::log::warn;
use bevy::prelude::{Vec2, Vec3};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub const NODE_HEIGHT_DISPLACE_HIGHPOLY_V2: &str = "Height Displace HighPoly V2";

const PARAM_HEIGHT_PATH: &str = "height_path";
const PARAM_STRENGTH: &str = "strength";
const PARAM_MID: &str = "mid";
const PARAM_SDF_REMESH: &str = "sdf_remesh";
const PARAM_VOXEL_SIZE: &str = "voxel_size";
const PARAM_BANDWIDTH: &str = "bandwidth";
const PARAM_ISO_VALUE: &str = "iso_value";
const PARAM_HARD_SURFACE: &str = "hard_surface";

#[derive(Default, Clone)]
pub struct HeightDisplaceHighPolyV2Node;

impl NodeParameters for HeightDisplaceHighPolyV2Node {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                PARAM_HEIGHT_PATH,
                "Height Path (optional)",
                "Input",
                ParameterValue::String(String::new()),
                ParameterUIType::FilePath { filters: vec!["png".into(), "jpg".into(), "jpeg".into(), "webp".into()] },
            ),
            Parameter::new(
                PARAM_STRENGTH,
                "Strength",
                "Displace",
                ParameterValue::Float(0.05),
                ParameterUIType::FloatSlider { min: 0.0, max: 1.0 },
            ),
            Parameter::new(
                PARAM_MID,
                "Mid",
                "Displace",
                ParameterValue::Float(0.5),
                ParameterUIType::FloatSlider { min: 0.0, max: 1.0 },
            ),
            Parameter::new(
                PARAM_SDF_REMESH,
                "SDF Remesh",
                "HighPoly",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                PARAM_VOXEL_SIZE,
                "Voxel Size",
                "HighPoly",
                ParameterValue::Float(0.01),
                ParameterUIType::FloatSlider { min: 0.0005, max: 0.2 },
            ),
            Parameter::new(
                PARAM_BANDWIDTH,
                "Bandwidth",
                "HighPoly",
                ParameterValue::Int(3),
                ParameterUIType::IntSlider { min: 1, max: 12 },
            ),
            Parameter::new(
                PARAM_ISO_VALUE,
                "Iso Value",
                "HighPoly",
                ParameterValue::Float(0.0),
                ParameterUIType::FloatSlider { min: -1.0, max: 1.0 },
            ),
            Parameter::new(
                PARAM_HARD_SURFACE,
                "Hard Surface",
                "HighPoly",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for HeightDisplaceHighPolyV2Node {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let input = inputs.get(0).map(|g| g.materialize()).unwrap_or_else(Geometry::new);
        Arc::new(compute_height_displace_highpoly_v2(&input, params))
    }
}

register_node!(NODE_HEIGHT_DISPLACE_HIGHPOLY_V2, "Modeling", HeightDisplaceHighPolyV2Node);

fn p_f32(params: &[Parameter], n: &str, d: f32) -> f32 {
    params.iter().find(|p| p.name == n).and_then(|p| if let ParameterValue::Float(v) = p.value { Some(v) } else { None }).unwrap_or(d)
}
fn p_i32(params: &[Parameter], n: &str, d: i32) -> i32 {
    params.iter().find(|p| p.name == n).and_then(|p| if let ParameterValue::Int(v) = p.value { Some(v) } else { None }).unwrap_or(d)
}
fn p_bool(params: &[Parameter], n: &str, d: bool) -> bool {
    params.iter().find(|p| p.name == n).and_then(|p| if let ParameterValue::Bool(v) = p.value { Some(v) } else { None }).unwrap_or(d)
}
fn p_str(params: &[Parameter], n: &str, d: &str) -> String {
    params.iter().find(|p| p.name == n).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None }).unwrap_or_else(|| d.to_string())
}

fn abs_path_from_assets_rel(rel: &str) -> Option<PathBuf> {
    let rel = rel.trim();
    if rel.is_empty() { return None; }
    let p0 = PathBuf::from(rel);
    if p0.is_absolute() { return Some(p0); }
    let cwd = std::env::current_dir().ok()?;
    Some(cwd.join("assets").join(rel))
}

fn height_path_from_geo_or_param(geo: &Geometry, params: &[Parameter]) -> Option<PathBuf> {
    let from_param = p_str(params, PARAM_HEIGHT_PATH, "");
    if !from_param.trim().is_empty() { return abs_path_from_assets_rel(from_param.as_str()); }
    let v = geo
        .get_detail_attribute(crate::nodes::ai_texture::ATTR_HEIGHTMAP_PATH)
        .and_then(|a: &Attribute| a.as_slice::<String>())
        .and_then(|v| v.first())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    abs_path_from_assets_rel(v.as_str())
}

fn approx_uv_eq(a: Vec2, b: Vec2) -> bool { (a - b).length_squared() <= 1e-10 }

fn ensure_point_uv(geo: &mut Geometry) -> Option<Vec<Vec2>> {
    if let Some(u) = geo.get_point_attribute(attrs::UV).and_then(|a: &Attribute| a.as_slice::<Vec2>()) {
        return Some(u.to_vec());
    }
    let uv_v: Vec<Vec2> = geo.get_vertex_attribute(attrs::UV).and_then(|a: &Attribute| a.as_slice::<Vec2>()).map(|s| s.to_vec())?;
    let p_len0 = geo.points().len();
    let mut puv: Vec<Option<Vec2>> = vec![None; p_len0];
    let pos = geo.get_point_attribute(attrs::P).and_then(|a: &Attribute| a.as_slice::<Vec3>())?.to_vec();

    // Split points so each point has a single UV.
    for prim in geo.primitives().values().into_iter().cloned().collect::<Vec<_>>() {
        let crate::libs::geometry::mesh::GeoPrimitive::Polygon(crate::libs::geometry::mesh::PolygonPrim { vertices }) = prim else { continue };
        for vid in vertices {
            let v = geo.vertices().get(vid.into())?;
            let vdi = geo.vertices().get_dense_index(vid.into())?;
            let uv = *uv_v.get(vdi)?;
            let pdi = geo.points().get_dense_index(v.point_id.into())?;
            match puv.get(pdi).and_then(|x| *x) {
                None => puv[pdi] = Some(uv),
                Some(u0) if approx_uv_eq(u0, uv) => {}
                Some(_) => {
                    let src_pid = v.point_id;
                    let src_pdi = pdi;
                    let new_pid = geo.add_point();
                    let new_pdi = geo.points().get_dense_index(new_pid.into())?;
                    // grow tracking
                    if new_pdi >= puv.len() { puv.resize(new_pdi + 1, None); }
                    puv[new_pdi] = Some(uv);
                    // copy @P (best-effort; keep other attrs default)
                    if let Some(p_mut) = geo.get_point_attribute_mut(attrs::P).and_then(|a: &mut Attribute| a.as_mut_slice::<Vec3>()) {
                        if let Some(vp) = pos.get(src_pdi).copied() { p_mut[new_pdi] = vp; }
                    }
                    geo.set_vertex_point(vid, new_pid);
                    // keep other vertices referencing src_pid unchanged
                    let _ = src_pid;
                }
            }
        }
    }

    let out: Vec<Vec2> = puv.into_iter().map(|u| u.unwrap_or(Vec2::ZERO)).collect();
    geo.insert_point_attribute(attrs::UV, Attribute::Vec2(out.clone()));
    Some(out)
}

fn n_per_point(mut geo: Geometry) -> Option<(Geometry, Vec<Vec3>)> {
    if let Some(n) = geo.get_point_attribute(attrs::N).and_then(|a: &Attribute| a.as_slice::<Vec3>()) {
        let nv = n.to_vec();
        return Some((geo, nv));
    }
    if geo.get_vertex_attribute(attrs::N).is_none() {
        geo.calculate_smooth_normals();
    }
    let n_v = geo.get_vertex_attribute(attrs::N).and_then(|a: &Attribute| a.as_slice::<Vec3>())?;
    let p_len = geo.points().len();
    let mut sum = vec![Vec3::ZERO; p_len];
    let mut cnt = vec![0u32; p_len];
    for prim in geo.primitives().values() {
        let crate::libs::geometry::mesh::GeoPrimitive::Polygon(crate::libs::geometry::mesh::PolygonPrim { vertices }) = prim else { continue };
        for &vid in vertices {
            let v = geo.vertices().get(vid.into())?;
            let pdi = geo.points().get_dense_index(v.point_id.into())?;
            let vdi = geo.vertices().get_dense_index(vid.into())?;
            let n = *n_v.get(vdi)?;
            sum[pdi] += n;
            cnt[pdi] += 1;
        }
    }
    let mut out = vec![Vec3::Y; p_len];
    for i in 0..p_len {
        if cnt[i] == 0 { return None; }
        out[i] = (sum[i] / (cnt[i] as f32)).normalize_or_zero();
        if out[i] == Vec3::ZERO { out[i] = Vec3::Y; }
    }
    Some((geo, out))
}

fn sample_height_luma8(img: &image::GrayImage, uv: Vec2) -> f32 {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 { return 0.0; }
    let u = uv.x.clamp(0.0, 1.0);
    let v = uv.y.clamp(0.0, 1.0);
    let x = u * (w as f32 - 1.0);
    let y = (1.0 - v) * (h as f32 - 1.0);
    let x0 = x.floor().clamp(0.0, (w - 1) as f32) as u32;
    let y0 = y.floor().clamp(0.0, (h - 1) as f32) as u32;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let s00 = img.get_pixel(x0, y0)[0] as f32 / 255.0;
    let s10 = img.get_pixel(x1, y0)[0] as f32 / 255.0;
    let s01 = img.get_pixel(x0, y1)[0] as f32 / 255.0;
    let s11 = img.get_pixel(x1, y1)[0] as f32 / 255.0;
    let a = s00 * (1.0 - tx) + s10 * tx;
    let b = s01 * (1.0 - tx) + s11 * tx;
    a * (1.0 - ty) + b * ty
}

pub fn compute_height_displace_highpoly_v2(input: &Geometry, params: &[Parameter]) -> Geometry {
    if input.get_point_attribute(attrs::P).and_then(|a: &Attribute| a.as_slice::<Vec3>()).is_none() { return Geometry::new(); }
    let mut geo = input.clone();
    let Some(_uvp) = ensure_point_uv(&mut geo) else {
        warn!("{NODE_HEIGHT_DISPLACE_HIGHPOLY_V2}: missing @uv (point or vertex).");
        return Geometry::new();
    };
    let Some(height_abs) = height_path_from_geo_or_param(input, params) else {
        warn!("{NODE_HEIGHT_DISPLACE_HIGHPOLY_V2}: missing height map path (param `{PARAM_HEIGHT_PATH}` or detail `{}`)", crate::nodes::ai_texture::ATTR_HEIGHTMAP_PATH);
        return Geometry::new();
    };
    let img = match image::open(&height_abs).map(|i| i.to_luma8()) {
        Ok(i) => i,
        Err(e) => {
            warn!("{NODE_HEIGHT_DISPLACE_HIGHPOLY_V2}: failed to load height image `{}`: {e}", height_abs.display());
            return Geometry::new();
        }
    };

    let strength = p_f32(params, PARAM_STRENGTH, 0.05).max(0.0);
    let mid = p_f32(params, PARAM_MID, 0.5).clamp(0.0, 1.0);
    let sdf_remesh = p_bool(params, PARAM_SDF_REMESH, true);

    let Some((mut geo, np)) = n_per_point(geo) else {
        warn!("{NODE_HEIGHT_DISPLACE_HIGHPOLY_V2}: missing normals (@N).");
        return Geometry::new();
    };

    let Some(p_attr) = geo.get_point_attribute(attrs::P).and_then(|a: &Attribute| a.as_slice::<Vec3>()) else { return Geometry::new(); };
    let Some(uvp) = geo.get_point_attribute(attrs::UV).and_then(|a: &Attribute| a.as_slice::<Vec2>()) else { return Geometry::new(); };
    let mut displaced: Vec<Vec3> = p_attr.to_vec();
    for i in 0..displaced.len().min(uvp.len()).min(np.len()) {
        let h01 = sample_height_luma8(&img, uvp[i]);
        let dh = (h01 - mid) * strength;
        displaced[i] += np[i] * dh;
    }

    geo.insert_point_attribute(attrs::P, Attribute::Vec3(displaced));

    if !sdf_remesh {
        return geo;
    }

    let voxel_size = p_f32(params, PARAM_VOXEL_SIZE, 0.01).max(1e-5);
    let bandwidth = p_i32(params, PARAM_BANDWIDTH, 3).clamp(1, 64);
    let iso_value = p_f32(params, PARAM_ISO_VALUE, 0.0);
    let hard_surface = p_bool(params, PARAM_HARD_SURFACE, false);

    let mut p = HashMap::new();
    p.insert("voxel_size".to_string(), ParameterValue::Float(voxel_size));
    p.insert("bandwidth".to_string(), ParameterValue::Int(bandwidth));
    p.insert("iso_value".to_string(), ParameterValue::Float(iso_value));
    p.insert("hard_surface".to_string(), ParameterValue::Bool(hard_surface));

    crate::nodes::modeling::sdf_remesh_v2::compute_sdf_remesh_v2(&geo, &p)
}

