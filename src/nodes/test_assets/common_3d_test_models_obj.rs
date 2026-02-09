//! Embedded OBJ test meshes with ensured @uv and normalized scale.

use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::{attrs, geo_ref::GeometryRef};
use crate::mesh::{Attribute, GeoPrimitive, Geometry, PolygonPrim};
use crate::{nodes::Parameter, register_node};
use bevy::prelude::{Vec2, Vec3};
use std::sync::{Arc, OnceLock};

#[inline]
fn f32_pair_or_zero(vt: &[Vec2], idx0: Option<i32>) -> Vec2 {
    let i = idx0.unwrap_or(-1);
    if i <= 0 { return Vec2::ZERO; }
    vt.get((i - 1) as usize).copied().unwrap_or(Vec2::ZERO)
}

#[inline]
fn uv_spherical(p: Vec3) -> Vec2 {
    let n = p.normalize_or_zero();
    let u = 0.5 + n.z.atan2(n.x) / (2.0 * std::f32::consts::PI);
    let v = 0.5 - n.y.asin() / std::f32::consts::PI;
    Vec2::new(u.rem_euclid(1.0), v.clamp(0.0, 1.0))
}

#[inline]
fn parse_embedded_obj(bytes: &[u8]) -> Geometry {
    puffin::profile_function!();
    let mut pos: Vec<Vec3> = Vec::with_capacity(1 << 15);
    let mut vt: Vec<Vec2> = Vec::new();
    let mut tris_vi: Vec<[i32; 3]> = Vec::with_capacity(1 << 16);
    let mut tris_vt: Vec<[i32; 3]> = Vec::with_capacity(1 << 16);

    for line in bytes.split(|&b| b == b'\n') {
        if line.len() < 2 { continue; }
        match line[0] {
            b'v' if line.get(1) == Some(&b' ') => {
                if let Ok(s) = std::str::from_utf8(&line[2..]) {
                    let mut it = s.split_whitespace();
                    let (Some(x), Some(y), Some(z)) = (it.next(), it.next(), it.next()) else { continue; };
                    if let (Ok(x), Ok(y), Ok(z)) = (x.parse::<f32>(), y.parse::<f32>(), z.parse::<f32>()) {
                        pos.push(Vec3::new(x, y, z));
                    }
                }
            }
            b'v' if line.get(1) == Some(&b't') && line.get(2) == Some(&b' ') => {
                if let Ok(s) = std::str::from_utf8(&line[3..]) {
                    let mut it = s.split_whitespace();
                    let (Some(u), Some(v)) = (it.next(), it.next()) else { continue; };
                    if let (Ok(u), Ok(v)) = (u.parse::<f32>(), v.parse::<f32>()) {
                        vt.push(Vec2::new(u, v));
                    }
                }
            }
            b'f' if line.get(1) == Some(&b' ') => {
                let Ok(s) = std::str::from_utf8(&line[2..]) else { continue; };
                let verts: Vec<&str> = s.split_whitespace().collect();
                if verts.len() < 3 { continue; }
                let mut parse_idx = |t: &str| -> (i32, Option<i32>) {
                    let mut it = t.split('/');
                    let vi = it.next().and_then(|x| x.parse::<i32>().ok()).unwrap_or(0);
                    let vti = it.next().and_then(|x| if x.is_empty() { None } else { x.parse::<i32>().ok() });
                    (vi, vti)
                };
                let (v0i, v0t) = parse_idx(verts[0]);
                for k in 1..(verts.len() - 1) {
                    let (v1i, v1t) = parse_idx(verts[k]);
                    let (v2i, v2t) = parse_idx(verts[k + 1]);
                    tris_vi.push([v0i, v1i, v2i]);
                    tris_vt.push([v0t.unwrap_or(-1), v1t.unwrap_or(-1), v2t.unwrap_or(-1)]);
                }
            }
            _ => {}
        }
    }

    if pos.is_empty() || tris_vi.is_empty() {
        return Geometry::new();
    }

    let mut mn = Vec3::splat(f32::INFINITY);
    let mut mx = Vec3::splat(f32::NEG_INFINITY);
    for p in pos.iter().copied() { mn = mn.min(p); mx = mx.max(p); }
    let c = (mn + mx) * 0.5;
    let ext = (mx - mn).max(Vec3::splat(1e-8));
    let maxe = ext.max_element();
    let scale = (100.0f32 * 0.01f32) / maxe; // normalize to ~1.0 max extent, keeps the explicit 0.01 requirement.
    for p in pos.iter_mut() { *p = (*p - c) * scale; }

    let mut geo = Geometry::new();
    let mut pids = Vec::with_capacity(pos.len());
    for _ in 0..pos.len() { pids.push(geo.add_point()); }
    geo.insert_point_attribute(attrs::P, Attribute::new(pos.clone()));

    let mut uvs: Vec<Vec2> = Vec::with_capacity(tris_vi.len() * 3);
    for (t, tt) in tris_vi.iter().zip(tris_vt.iter()) {
        let mut vids = Vec::with_capacity(3);
        for j in 0..3 {
            let vi = t[j];
            let idx = if vi < 0 { (pos.len() as i32 + vi) as usize } else { (vi - 1).max(0) as usize };
            let pid = *pids.get(idx).unwrap_or(&pids[0]);
            let vid = geo.add_vertex(pid);
            vids.push(vid);
            let uv = if tt[j] > 0 { f32_pair_or_zero(&vt, Some(tt[j])) } else { uv_spherical(pos[idx]) };
            uvs.push(uv);
        }
        geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: vids }));
    }
    geo.insert_vertex_attribute(attrs::UV, Attribute::new(uvs));
    geo.calculate_flat_normals();
    geo
}

macro_rules! test_obj_node {
    ($ty:ident, $name:expr, $file:expr) => {
        #[derive(Default)]
        pub struct $ty;

        impl NodeParameters for $ty {
            fn define_parameters() -> Vec<Parameter> { Vec::new() }
        }

        impl NodeOp for $ty {
            fn compute(
                &self,
                _params: &[Parameter],
                _inputs: &[Arc<dyn GeometryRef>],
            ) -> Arc<Geometry> {
                static GEO: OnceLock<Arc<Geometry>> = OnceLock::new();
                const BYTES: &[u8] = include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/assets/models/common-3d-test-models/",
                    $file
                ));
                GEO.get_or_init(|| Arc::new(parse_embedded_obj(BYTES))).clone()
            }
        }

        register_node!($name, "Test Assets", $ty);
    };
}

test_obj_node!(CreateAlligatorObjNode, "Create Alligator (OBJ)", "alligator.obj");
test_obj_node!(CreateArmadilloObjNode, "Create Armadillo (OBJ)", "armadillo.obj");
test_obj_node!(CreateBeastObjNode, "Create Beast (OBJ)", "beast.obj");
test_obj_node!(CreateBeetleObjNode, "Create Beetle (OBJ)", "beetle.obj");
test_obj_node!(CreateBeetleAltObjNode, "Create Beetle Alt (OBJ)", "beetle-alt.obj");
test_obj_node!(CreateBimbaObjNode, "Create Bimba (OBJ)", "bimba.obj");
test_obj_node!(CreateCheburashkaObjNode, "Create Cheburashka (OBJ)", "cheburashka.obj");
test_obj_node!(CreateCowObjNode, "Create Cow (OBJ)", "cow.obj");
test_obj_node!(CreateFandiskObjNode, "Create Fandisk (OBJ)", "fandisk.obj");
test_obj_node!(CreateHappyObjNode, "Create Happy Buddha (OBJ)", "happy.obj");
test_obj_node!(CreateHomerObjNode, "Create Homer (OBJ)", "homer.obj");
test_obj_node!(CreateHorseObjNode, "Create Horse (OBJ)", "horse.obj");
test_obj_node!(CreateIgeaObjNode, "Create Igea (OBJ)", "igea.obj");
test_obj_node!(CreateLucyObjNode, "Create Lucy (OBJ)", "lucy.obj");
test_obj_node!(CreateMaxPlanckObjNode, "Create Max Planck (OBJ)", "max-planck.obj");
test_obj_node!(CreateNefertitiObjNode, "Create Nefertiti (OBJ)", "nefertiti.obj");
test_obj_node!(CreateOgreObjNode, "Create Ogre (OBJ)", "ogre.obj");
test_obj_node!(CreateRockerArmObjNode, "Create Rocker Arm (OBJ)", "rocker-arm.obj");
test_obj_node!(CreateSpotObjNode, "Create Spot (OBJ)", "spot.obj");
test_obj_node!(CreateStanfordBunnyObjNode, "Create Stanford Bunny (OBJ)", "stanford-bunny.obj");
test_obj_node!(CreateSuzanneObjNode, "Create Suzanne (OBJ)", "suzanne.obj");
test_obj_node!(CreateTeapotObjNode, "Create Teapot (OBJ)", "teapot.obj");
test_obj_node!(CreateWoodyObjNode, "Create Woody (OBJ)", "woody.obj");
test_obj_node!(CreateXyzrgbDragonObjNode, "Create XYZRGB Dragon (OBJ)", "xyzrgb_dragon.obj");

