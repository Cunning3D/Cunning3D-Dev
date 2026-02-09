use crate::libs::algorithms::algorithms_dcc::PagedBuffer;
use crate::libs::geometry::ids::{AttributeId, PrimId};
use crate::libs::geometry::mesh::{
    Attribute, GeoPrimitive, GeoVertex, Geometry, PolygonPrim, PolylinePrim, PrimitiveType,
};
use crate::libs::geometry::topology::Topology;
use bevy::prelude::*;
use rhai::{Dynamic, Engine};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct HudContext {
    pub commands: Arc<Mutex<Vec<(String, String)>>>,
    pub param_updates: Arc<Mutex<Vec<(String, Dynamic)>>>,
    pub state_updates: Arc<Mutex<Vec<(String, Dynamic)>>>,
}

impl HudContext {
    pub fn new() -> Self {
        Self {
            commands: Arc::new(Mutex::new(Vec::new())),
            param_updates: Arc::new(Mutex::new(Vec::new())),
            state_updates: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[derive(Clone, Debug)]
pub struct InputContext {
    pub ray_origin: Vec3,
    pub ray_direction: Vec3,
    pub cursor_pos: Vec2,
    pub mouse_left_pressed: bool,
    pub mouse_left_just_pressed: bool,
    pub mouse_left_just_released: bool,
    pub cam_pos: Vec3,
    pub is_orthographic: bool,
    pub scale_factor: f32,
}

#[derive(Clone, Debug)]
pub enum GizmoScriptCommand {
    Line {
        start: Vec3,
        end: Vec3,
        color: Vec3,
    },
    Sphere {
        center: Vec3,
        radius: f32,
        color: Vec3,
    },
    Cube {
        center: Vec3,
        size: f32,
        color: Vec3,
    },
}

#[derive(Clone, Debug)]
pub struct GizmoScriptContext {
    pub commands: Arc<Mutex<Vec<GizmoScriptCommand>>>,
}

impl GizmoScriptContext {
    pub fn new() -> Self {
        Self {
            commands: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

pub fn register_api(engine: &mut Engine) {
    // 1. Basic Types
    engine
        .register_type_with_name::<Vec3>("Vec3")
        .register_fn("vec3", |x: f64, y: f64, z: f64| {
            Vec3::new(x as f32, y as f32, z as f32)
        })
        .register_fn("print", |s: &str| info!("Rhai: {}", s))
        .register_fn("log_info", |s: &str| info!("Rhai Info: {}", s))
        .register_fn("debug", |v: Vec3| info!("Vec3({}, {}, {})", v.x, v.y, v.z));

    engine
        .register_type_with_name::<Vec2>("Vec2")
        .register_fn("vec2", |x: f64, y: f64| Vec2::new(x as f32, y as f32))
        .register_get("x", |v: &mut Vec2| v.x as f64)
        .register_get("y", |v: &mut Vec2| v.y as f64)
        .register_set("x", |v: &mut Vec2, val: f64| v.x = val as f32)
        .register_set("y", |v: &mut Vec2, val: f64| v.y = val as f32);

    // HUD API
    engine
        .register_type_with_name::<HudContext>("HudContext")
        .register_fn("label", |ctx: &mut HudContext, text: &str| {
            if let Ok(mut cmds) = ctx.commands.lock() {
                cmds.push(("label".to_string(), text.to_string()));
            }
        })
        .register_fn("heading", |ctx: &mut HudContext, text: &str| {
            if let Ok(mut cmds) = ctx.commands.lock() {
                cmds.push(("heading".to_string(), text.to_string()));
            }
        })
        .register_fn(
            "set_param",
            |ctx: &mut HudContext, name: &str, value: Dynamic| {
                if let Ok(mut ops) = ctx.param_updates.lock() {
                    ops.push((name.to_string(), value));
                }
            },
        )
        .register_fn(
            "set_state",
            |ctx: &mut HudContext, key: &str, value: Dynamic| {
                if let Ok(mut ops) = ctx.state_updates.lock() {
                    ops.push((key.to_string(), value));
                }
            },
        );

    // Input / Gizmo API
    engine
        .register_type_with_name::<InputContext>("InputContext")
        .register_fn("ray_origin", |ctx: &mut InputContext| ctx.ray_origin)
        .register_fn("ray_direction", |ctx: &mut InputContext| ctx.ray_direction)
        .register_fn("cursor_pos", |ctx: &mut InputContext| ctx.cursor_pos)
        .register_fn("mouse_left_pressed", |ctx: &mut InputContext| {
            ctx.mouse_left_pressed
        })
        .register_fn("mouse_left_just_pressed", |ctx: &mut InputContext| {
            ctx.mouse_left_just_pressed
        })
        .register_fn("mouse_left_just_released", |ctx: &mut InputContext| {
            ctx.mouse_left_just_released
        })
        .register_fn("cam_pos", |ctx: &mut InputContext| ctx.cam_pos)
        .register_fn("is_orthographic", |ctx: &mut InputContext| {
            ctx.is_orthographic
        })
        .register_fn("scale_factor", |ctx: &mut InputContext| ctx.scale_factor);

    engine
        .register_type_with_name::<GizmoScriptContext>("GizmoContext")
        .register_fn(
            "line",
            |ctx: &mut GizmoScriptContext, start: Vec3, end: Vec3, color: Vec3| {
                if let Ok(mut cmds) = ctx.commands.lock() {
                    cmds.push(GizmoScriptCommand::Line { start, end, color });
                }
            },
        )
        .register_fn(
            "sphere",
            |ctx: &mut GizmoScriptContext, center: Vec3, radius: f64, color: Vec3| {
                if let Ok(mut cmds) = ctx.commands.lock() {
                    cmds.push(GizmoScriptCommand::Sphere {
                        center,
                        radius: radius as f32,
                        color,
                    });
                }
            },
        )
        .register_fn(
            "cube",
            |ctx: &mut GizmoScriptContext, center: Vec3, size: f64, color: Vec3| {
                if let Ok(mut cmds) = ctx.commands.lock() {
                    cmds.push(GizmoScriptCommand::Cube {
                        center,
                        size: size as f32,
                        color,
                    });
                }
            },
        );

    // Vec3 Operations
    engine.register_fn("+", |a: Vec3, b: Vec3| a + b);
    engine.register_fn("-", |a: Vec3, b: Vec3| a - b);
    engine.register_fn("*", |a: Vec3, f: f64| a * (f as f32));
    engine.register_fn("/", |a: Vec3, f: f64| a / (f as f32));
    engine.register_get("x", |v: &mut Vec3| v.x as f64);
    engine.register_get("y", |v: &mut Vec3| v.y as f64);
    engine.register_get("z", |v: &mut Vec3| v.z as f64);
    engine.register_set("x", |v: &mut Vec3, val: f64| v.x = val as f32);
    engine.register_set("y", |v: &mut Vec3, val: f64| v.y = val as f32);
    engine.register_set("z", |v: &mut Vec3, val: f64| v.z = val as f32);

    // Vec3 math helpers
    engine.register_fn("dot", |a: Vec3, b: Vec3| a.dot(b) as f64);
    engine.register_fn("cross", |a: Vec3, b: Vec3| a.cross(b));
    engine.register_fn("length", |v: Vec3| v.length() as f64);
    engine.register_fn("normalize", |v: Vec3| {
        if v.length_squared() > 0.0 {
            v.normalize()
        } else {
            Vec3::ZERO
        }
    });

    // 2. Geometry API
    engine.register_type_with_name::<Geometry>("Geometry");
    engine.register_fn("new_geometry", || Geometry::new());

    // Basic Geometry Query
    engine.register_fn("point_count", |geo: &mut Geometry| {
        geo.get_point_count() as i64
    });
    engine.register_fn("vertex_count", |geo: &mut Geometry| {
        geo.vertices().len() as i64
    });
    engine.register_fn("primitive_count", |geo: &mut Geometry| {
        geo.primitives().len() as i64
    });

    engine.register_fn(
        "get_vertex_point_index",
        |geo: &mut Geometry, idx: i64| -> i64 {
            if idx < 0 {
                return -1;
            }
            if let Some(vid) = geo.vertices().get_id_from_dense(idx as usize) {
                if let Some(v) = geo.vertices().get(vid) {
                    return geo
                        .points()
                        .get_dense_index(v.point_id.into())
                        .map(|i| i as i64)
                        .unwrap_or(-1);
                }
            }
            -1
        },
    );

    engine.register_fn(
        "get_primitive_vertices",
        |geo: &mut Geometry, prim_idx: i64| -> rhai::Array {
            let mut arr = rhai::Array::new();
            if prim_idx < 0 {
                return arr;
            }
            if let Some(prim_id) = geo.primitives().get_id_from_dense(prim_idx as usize) {
                if let Some(prim) = geo.primitives().get(prim_id) {
                    for &vid in prim.vertices() {
                        if let Some(v_dense) = geo.vertices().get_dense_index(vid.into()) {
                            arr.push(rhai::Dynamic::from(v_dense as i64));
                        }
                    }
                }
            }
            arr
        },
    );

    // Point Manipulation
    engine.register_fn("get_point_pos", |geo: &mut Geometry, idx: i64| -> Vec3 {
        if let Some(positions) = geo.get_point_position_attribute() {
            if idx >= 0 && (idx as usize) < positions.len() {
                return positions[idx as usize];
            }
        }
        Vec3::ZERO
    });

    engine.register_fn(
        "set_point_pos",
        |geo: &mut Geometry, idx: i64, pos: Vec3| {
            if let Some(positions) = geo
                .get_point_attribute_mut("@P")
                .and_then(|a| a.as_storage_mut::<Vec<Vec3>>())
            {
                if idx >= 0 && (idx as usize) < positions.len() {
                    if let Some(p) = positions.get_mut(idx as usize) {
                        *p = pos;
                    }
                }
            }
        },
    );

    engine.register_fn("add_point", |geo: &mut Geometry, pos: Vec3| -> i64 {
        let pid = geo.add_point();
        let dense_idx = geo.points().get_dense_index(pid.into());

        // Set position. add_point defaults to 0.0.
        if let Some(idx) = dense_idx {
            if let Some(positions) = geo
                .get_point_attribute_mut("@P")
                .and_then(|a| a.as_storage_mut::<Vec<Vec3>>())
            {
                if idx < positions.len() {
                    positions[idx] = pos;
                }
            }
        }

        let vid = geo.add_vertex(pid);

        dense_idx.map(|i| i as i64).unwrap_or(-1)
    });

    engine.register_fn(
        "get_point_attrib_f32",
        |geo: &mut Geometry, name: &str, idx: i64| -> f64 {
            if idx < 0 {
                return 0.0;
            }
            if let Some(values) = geo
                .get_point_attribute(name)
                .and_then(|a| a.as_slice::<f32>())
            {
                if let Some(v) = values.get(idx as usize) {
                    return *v as f64;
                }
            }
            0.0
        },
    );

    engine.register_fn(
        "get_point_attrib_vec3",
        |geo: &mut Geometry, name: &str, idx: i64| -> Vec3 {
            if idx < 0 {
                return Vec3::ZERO;
            }
            if let Some(values) = geo
                .get_point_attribute(name)
                .and_then(|a| a.as_slice::<Vec3>())
            {
                if let Some(v) = values.get(idx as usize) {
                    return *v;
                }
            }
            Vec3::ZERO
        },
    );

    engine.register_fn(
        "get_point_attrib_i32",
        |geo: &mut Geometry, name: &str, idx: i64| -> i64 {
            if idx < 0 {
                return 0;
            }
            if let Some(values) = geo
                .get_point_attribute(name)
                .and_then(|a| a.as_slice::<i32>())
            {
                if let Some(v) = values.get(idx as usize) {
                    return *v as i64;
                }
            }
            0
        },
    );

    engine.register_fn(
        "get_point_attrib_bool",
        |geo: &mut Geometry, name: &str, idx: i64| -> bool {
            if idx < 0 {
                return false;
            }
            if let Some(values) = geo
                .get_point_attribute(name)
                .and_then(|a| a.as_slice::<bool>())
            {
                if let Some(v) = values.get(idx as usize) {
                    return *v;
                }
            }
            false
        },
    );

    engine.register_fn(
        "get_point_attrib_string",
        |geo: &mut Geometry, name: &str, idx: i64| -> String {
            if idx < 0 {
                return "".to_string();
            }
            if let Some(values) = geo
                .get_point_attribute(name)
                .and_then(|a| a.as_slice::<String>())
            {
                if let Some(v) = values.get(idx as usize) {
                    return v.clone();
                }
            }
            "".to_string()
        },
    );

    engine.register_fn(
        "get_vertex_attrib_f32",
        |geo: &mut Geometry, name: &str, idx: i64| -> f64 {
            if idx < 0 {
                return 0.0;
            }
            if let Some(values) = geo
                .get_vertex_attribute(name)
                .and_then(|a| a.as_slice::<f32>())
            {
                if let Some(v) = values.get(idx as usize) {
                    return *v as f64;
                }
            }
            0.0
        },
    );

    engine.register_fn(
        "get_vertex_attrib_vec3",
        |geo: &mut Geometry, name: &str, idx: i64| -> Vec3 {
            if idx < 0 {
                return Vec3::ZERO;
            }
            if let Some(values) = geo
                .get_vertex_attribute(name)
                .and_then(|a| a.as_slice::<Vec3>())
            {
                if let Some(v) = values.get(idx as usize) {
                    return *v;
                }
            }
            Vec3::ZERO
        },
    );

    engine.register_fn(
        "get_primitive_attrib_f32",
        |geo: &mut Geometry, name: &str, idx: i64| -> f64 {
            if idx < 0 {
                return 0.0;
            }
            if let Some(values) = geo
                .get_primitive_attribute(name)
                .and_then(|a| a.as_slice::<f32>())
            {
                if let Some(v) = values.get(idx as usize) {
                    return *v as f64;
                }
            }
            0.0
        },
    );

    engine.register_fn(
        "get_primitive_attrib_vec3",
        |geo: &mut Geometry, name: &str, idx: i64| -> Vec3 {
            if idx < 0 {
                return Vec3::ZERO;
            }
            if let Some(values) = geo
                .get_primitive_attribute(name)
                .and_then(|a| a.as_slice::<Vec3>())
            {
                if let Some(v) = values.get(idx as usize) {
                    return *v;
                }
            }
            Vec3::ZERO
        },
    );

    engine.register_fn(
        "get_detail_attrib_f32",
        |geo: &mut Geometry, name: &str| -> f64 {
            if let Some(values) = geo
                .get_detail_attribute(name)
                .and_then(|a| a.as_slice::<f32>())
            {
                if let Some(v) = values.get(0) {
                    return *v as f64;
                }
            }
            0.0
        },
    );

    // Setters needing Vec push/resize
    engine.register_fn(
        "set_point_attrib_f32",
        |geo: &mut Geometry, name: &str, idx: i64, val: f64| {
            if idx < 0 {
                return;
            }
            let target_index = idx as usize;
            if geo.get_point_attribute(name).is_none() {
                geo.insert_point_attribute(name, Attribute::new(PagedBuffer::<f32>::new()));
            }
            if let Some(values) = geo
                .get_point_attribute_mut(name)
                .and_then(|a| a.as_storage_mut::<Vec<f32>>())
            {
                while values.len() <= target_index {
                    values.push(0.0);
                }
                if let Some(v) = values.get_mut(target_index) {
                    *v = val as f32;
                }
            }
        },
    );

    engine.register_fn(
        "set_point_attrib_vec3",
        |geo: &mut Geometry, name: &str, idx: i64, val: Vec3| {
            if idx < 0 {
                return;
            }
            let target_index = idx as usize;
            if geo.get_point_attribute(name).is_none() {
                geo.insert_point_attribute(name, Attribute::new(PagedBuffer::<Vec3>::new()));
            }
            if let Some(values) = geo
                .get_point_attribute_mut(name)
                .and_then(|a| a.as_storage_mut::<Vec<Vec3>>())
            {
                while values.len() <= target_index {
                    values.push(Vec3::ZERO);
                }
                if let Some(v) = values.get_mut(target_index) {
                    *v = val;
                }
            }
        },
    );

    engine.register_fn(
        "set_point_attrib_i32",
        |geo: &mut Geometry, name: &str, idx: i64, val: i64| {
            if idx < 0 {
                return;
            }
            let target_index = idx as usize;
            if geo.get_point_attribute(name).is_none() {
                geo.insert_point_attribute(name, Attribute::new(PagedBuffer::<i32>::new()));
            }
            if let Some(values) = geo
                .get_point_attribute_mut(name)
                .and_then(|a| a.as_storage_mut::<Vec<i32>>())
            {
                while values.len() <= target_index {
                    values.push(0);
                }
                if let Some(v) = values.get_mut(target_index) {
                    *v = val as i32;
                }
            }
        },
    );

    engine.register_fn(
        "set_point_attrib_bool",
        |geo: &mut Geometry, name: &str, idx: i64, val: bool| {
            if idx < 0 {
                return;
            }
            let target_index = idx as usize;
            if geo.get_point_attribute(name).is_none() {
                geo.insert_point_attribute(name, Attribute::new(PagedBuffer::<bool>::new()));
            }
            if let Some(values) = geo
                .get_point_attribute_mut(name)
                .and_then(|a| a.as_storage_mut::<Vec<bool>>())
            {
                while values.len() <= target_index {
                    values.push(false);
                }
                if let Some(v) = values.get_mut(target_index) {
                    *v = val;
                }
            }
        },
    );

    engine.register_fn(
        "set_point_attrib_string",
        |geo: &mut Geometry, name: &str, idx: i64, val: &str| {
            if idx < 0 {
                return;
            }
            let target_index = idx as usize;
            if geo.get_point_attribute(name).is_none() {
                geo.insert_point_attribute(name, Attribute::new(PagedBuffer::<String>::new()));
            }
            if let Some(values) = geo
                .get_point_attribute_mut(name)
                .and_then(|a| a.as_storage_mut::<Vec<String>>())
            {
                while values.len() <= target_index {
                    values.push(String::new());
                }
                if let Some(v) = values.get_mut(target_index) {
                    *v = val.to_string();
                }
            }
        },
    );

    // --- Group API ---
    engine.register_fn("ensure_point_group", |geo: &mut Geometry, name: &str| {
        geo.ensure_point_group(name);
    });

    engine.register_fn(
        "set_point_group",
        |geo: &mut Geometry, name: &str, idx: i64, val: bool| {
            if idx < 0 {
                return;
            }
            if let Some(mask) = geo.get_point_group_mut(name) {
                if (idx as usize) < mask.len() {
                    mask.set(idx as usize, val);
                }
            }
        },
    );

    engine.register_fn(
        "get_point_group",
        |geo: &mut Geometry, name: &str, idx: i64| -> bool {
            if idx < 0 {
                return false;
            }
            if let Some(mask) = geo.get_point_group(name) {
                if (idx as usize) < mask.len() {
                    return mask.get(idx as usize);
                }
            }
            false
        },
    );

    engine.register_fn(
        "ensure_primitive_group",
        |geo: &mut Geometry, name: &str| {
            geo.ensure_primitive_group(name);
        },
    );

    engine.register_fn(
        "set_primitive_group",
        |geo: &mut Geometry, name: &str, idx: i64, val: bool| {
            if idx < 0 {
                return;
            }
            if let Some(mask) = geo.get_primitive_group_mut(name) {
                if (idx as usize) < mask.len() {
                    mask.set(idx as usize, val);
                }
            }
        },
    );

    engine.register_fn(
        "get_primitive_group",
        |geo: &mut Geometry, name: &str, idx: i64| -> bool {
            if idx < 0 {
                return false;
            }
            if let Some(mask) = geo.get_primitive_group(name) {
                if (idx as usize) < mask.len() {
                    return mask.get(idx as usize);
                }
            }
            false
        },
    );

    // --- Topology API ---
    engine.register_type_with_name::<Topology>("Topology");

    engine.register_fn("build_topology", |geo: &Geometry| -> Topology {
        geo.build_topology().as_ref().clone()
    });

    engine.register_fn(
        "get_point_neighbors",
        |topo: &mut Topology, pt_idx: i64| -> rhai::Array {
            let mut arr = rhai::Array::new();
            if pt_idx < 0 {
                return arr;
            }
            use crate::libs::geometry::ids::{GenerationalId, PointId};
            let pid = PointId::from_raw(pt_idx as u32, 0); // UNSAFE!
            for n in topo.get_point_neighbors(pid) {
                arr.push(rhai::Dynamic::from(n.index() as i64));
            }
            arr
        },
    );

    engine.register_fn(
        "get_primitive_neighbors",
        |topo: &mut Topology, prim_idx: i64| -> rhai::Array {
            let mut arr = rhai::Array::new();
            if prim_idx < 0 {
                return arr;
            }
            use crate::libs::geometry::ids::{GenerationalId, PrimId};
            let pid = PrimId::from_raw(prim_idx as u32, 0); // UNSAFE!
            for n in topo.get_primitive_neighbors(pid) {
                arr.push(rhai::Dynamic::from(n.index() as i64));
            }
            arr
        },
    );

    engine.register_fn(
        "set_vertex_attrib_f32",
        |geo: &mut Geometry, name: &str, idx: i64, val: f64| {
            if idx < 0 {
                return;
            }
            let target_index = idx as usize;
            if geo.get_vertex_attribute(name).is_none() {
                geo.insert_vertex_attribute(name, Attribute::new(PagedBuffer::<f32>::new()));
            }
            if let Some(values) = geo
                .get_vertex_attribute_mut(name)
                .and_then(|a| a.as_storage_mut::<Vec<f32>>())
            {
                while values.len() <= target_index {
                    values.push(0.0);
                }
                if let Some(v) = values.get_mut(target_index) {
                    *v = val as f32;
                }
            }
        },
    );

    engine.register_fn(
        "set_vertex_attrib_vec3",
        |geo: &mut Geometry, name: &str, idx: i64, val: Vec3| {
            if idx < 0 {
                return;
            }
            let target_index = idx as usize;
            if geo.get_vertex_attribute(name).is_none() {
                geo.insert_vertex_attribute(name, Attribute::new(PagedBuffer::<Vec3>::new()));
            }
            if let Some(values) = geo
                .get_vertex_attribute_mut(name)
                .and_then(|a| a.as_storage_mut::<Vec<Vec3>>())
            {
                while values.len() <= target_index {
                    values.push(Vec3::ZERO);
                }
                if let Some(v) = values.get_mut(target_index) {
                    *v = val;
                }
            }
        },
    );

    engine.register_fn(
        "set_primitive_attrib_f32",
        |geo: &mut Geometry, name: &str, idx: i64, val: f64| {
            if idx < 0 {
                return;
            }
            let target_index = idx as usize;
            if geo.get_primitive_attribute(name).is_none() {
                geo.insert_primitive_attribute(name, Attribute::new(PagedBuffer::<f32>::new()));
            }
            if let Some(values) = geo
                .get_primitive_attribute_mut(name)
                .and_then(|a| a.as_storage_mut::<Vec<f32>>())
            {
                while values.len() <= target_index {
                    values.push(0.0);
                }
                if let Some(v) = values.get_mut(target_index) {
                    *v = val as f32;
                }
            }
        },
    );

    engine.register_fn(
        "set_primitive_attrib_vec3",
        |geo: &mut Geometry, name: &str, idx: i64, val: Vec3| {
            if idx < 0 {
                return;
            }
            let target_index = idx as usize;
            if geo.get_primitive_attribute(name).is_none() {
                geo.insert_primitive_attribute(name, Attribute::new(PagedBuffer::<Vec3>::new()));
            }
            if let Some(values) = geo
                .get_primitive_attribute_mut(name)
                .and_then(|a| a.as_storage_mut::<Vec<Vec3>>())
            {
                while values.len() <= target_index {
                    values.push(Vec3::ZERO);
                }
                if let Some(v) = values.get_mut(target_index) {
                    *v = val;
                }
            }
        },
    );

    engine.register_fn(
        "set_detail_attrib_f32",
        |geo: &mut Geometry, name: &str, val: f64| {
            if geo.get_detail_attribute(name).is_none() {
                geo.insert_detail_attribute(name, Attribute::new(PagedBuffer::<f32>::new()));
            }
            if let Some(values) = geo
                .get_detail_attribute_mut(name)
                .and_then(|a| a.as_storage_mut::<Vec<f32>>())
            {
                if values.is_empty() {
                    values.push(val as f32);
                } else {
                    if let Some(v) = values.get_mut(0) {
                        *v = val as f32;
                    }
                }
            }
        },
    );

    engine.register_fn("remove_point_attrib", |geo: &mut Geometry, name: &str| {
        geo.remove_point_attribute(name);
    });

    engine.register_fn("remove_vertex_attrib", |geo: &mut Geometry, name: &str| {
        geo.remove_vertex_attribute(name);
    });

    engine.register_fn(
        "remove_primitive_attrib",
        |geo: &mut Geometry, name: &str| {
            geo.remove_primitive_attribute(name);
        },
    );

    engine.register_fn("remove_detail_attrib", |geo: &mut Geometry, name: &str| {
        geo.remove_detail_attribute(name);
    });

    engine.register_fn("remove_primitive", |geo: &mut Geometry, prim_idx: i64| {
        if prim_idx < 0 {
            return;
        }
        let idx = prim_idx as usize;
        if let Some(id) = geo.primitives().get_id_from_dense(idx) {
            geo.remove_primitive(PrimId::from(id));
        }
    });

    engine.register_fn(
        "add_polyline_primitive",
        |geo: &mut Geometry, indices: rhai::Array| -> i64 {
            let mut verts: Vec<usize> = Vec::new();
            for v in indices {
                if let Some(i) = v.clone().try_cast::<i64>() {
                    if i >= 0 {
                        verts.push(i as usize);
                    }
                }
            }
            let mut poly_verts = Vec::new();
            for v_idx in verts {
                if let Some(vid) = geo.get_vertex_id_at_dense_index(v_idx) {
                    poly_verts.push(vid);
                }
            }
            if poly_verts.len() >= 2 {
                let pid = geo.add_primitive(GeoPrimitive::Polyline(PolylinePrim {
                    vertices: poly_verts,
                    closed: false,
                }));
                geo.primitives()
                    .get_dense_index(pid.into())
                    .map(|i| i as i64)
                    .unwrap_or(-1)
            } else {
                -1
            }
        },
    );

    engine.register_fn(
        "add_polygon_primitive",
        |geo: &mut Geometry, indices: rhai::Array| -> i64 {
            let mut verts: Vec<usize> = Vec::new();
            for v in indices {
                if let Some(i) = v.clone().try_cast::<i64>() {
                    if i >= 0 {
                        verts.push(i as usize);
                    }
                }
            }
            let mut poly_verts = Vec::new();
            for v_idx in verts {
                if let Some(vid) = geo.get_vertex_id_at_dense_index(v_idx) {
                    poly_verts.push(vid);
                }
            }
            if poly_verts.len() >= 3 {
                let pid = geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim {
                    vertices: poly_verts,
                }));
                geo.primitives()
                    .get_dense_index(pid.into())
                    .map(|i| i as i64)
                    .unwrap_or(-1)
            } else {
                -1
            }
        },
    );
}
