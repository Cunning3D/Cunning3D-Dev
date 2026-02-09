use crate::cunning_core::scripting::{ScriptEngine, ScriptNodeState};
use crate::cunning_core::traits::node_interface::{
    GizmoContext, GizmoDrawBuffer, GizmoPrimitive, GizmoState, NodeInteraction, NodeOp,
    ServiceProvider,
};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::Geometry;
use crate::nodes::parameter::Parameter;
use crate::nodes::NodeGraphResource;
use crate::nodes::graph_model::{enqueue_graph_command, GraphCommandEffect};
use bevy::prelude::*;
use rhai::{Dynamic, Engine, Scope, AST};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone, Debug)]
enum ParamUpdateValue {
    Float(f32),
    Int(i32),
    Bool(bool),
    Vec3(Vec3),
    String(String),
}

fn dynamic_to_param_update_value(v: &Dynamic) -> Option<ParamUpdateValue> {
    if let Some(x) = v.clone().try_cast::<f64>() {
        return Some(ParamUpdateValue::Float(x as f32));
    }
    if let Some(x) = v.clone().try_cast::<i64>() {
        return Some(ParamUpdateValue::Int(x as i32));
    }
    if let Some(x) = v.clone().try_cast::<bool>() {
        return Some(ParamUpdateValue::Bool(x));
    }
    if let Some(x) = v.clone().try_cast::<Vec3>() {
        return Some(ParamUpdateValue::Vec3(x));
    }
    if let Some(x) = v.clone().try_cast::<String>() {
        return Some(ParamUpdateValue::String(x));
    }
    None
}

/// Represents a runtime instance of a Rhai script node.
/// It holds the pre-compiled AST so we don't recompile every frame.
#[derive(Clone)]
pub struct RhaiNodeOp {
    pub script_name: String,
    pub ast: Arc<AST>,
    pub ui_ast: Option<Arc<AST>>,
    pub engine: ScriptEngine, // Shared reference to the engine
}

impl RhaiNodeOp {
    pub fn new(name: &str, ast: AST, ui_ast: Option<Arc<AST>>, engine: ScriptEngine) -> Self {
        Self {
            script_name: name.to_string(),
            ast: Arc::new(ast),
            ui_ast,
            engine,
        }
    }
}

impl NodeOp for RhaiNodeOp {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        println!("RhaiNodeOp::compute: Start for {}", self.script_name);
        let engine_lock = match self.engine.0.lock() {
            Ok(guard) => guard,
            Err(e) => {
                error!("Rhai Engine Lock Poisoned: {}", e);
                return Arc::new(Geometry::new());
            }
        };

        let mut scope = Scope::new();

        // 1. Bind Inputs
        let primary_geo = inputs
            .first()
            .map(|g| g.materialize())
            .unwrap_or_else(Geometry::new);
        println!(
            "RhaiNodeOp::compute: Input Geo Points: {}",
            primary_geo.get_point_count()
        );

        // Also expose to scope so scripts can read them as globals if they want
        let mut inputs_array = rhai::Array::new();
        for input_geo in inputs {
            inputs_array.push(Dynamic::from(input_geo.materialize()));
        }

        scope.push("geo", primary_geo.clone());
        scope.push("inputs", inputs_array.clone());

        // 2. Bind Parameters
        let mut params_map = rhai::Map::new();
        for p in params {
            use crate::nodes::parameter::{CurveType, ParameterValue};
            let val = match &p.value {
                ParameterValue::Float(f) => Dynamic::from(*f as f64),
                ParameterValue::Int(i) => Dynamic::from(*i),
                ParameterValue::String(s) => Dynamic::from(s.clone()),
                ParameterValue::Bool(b) => Dynamic::from(*b),
                ParameterValue::Vec3(v) => Dynamic::from(*v),
                ParameterValue::Curve(data) => {
                    let mut curve_map = rhai::Map::new();

                    let mut points_array = rhai::Array::new();
                    for pt in &data.points {
                        let mut pt_map = rhai::Map::new();
                        pt_map.insert("id".into(), Dynamic::from(pt.id.to_string()));
                        pt_map.insert("position".into(), Dynamic::from(pt.position));
                        pt_map.insert("handle_in".into(), Dynamic::from(pt.handle_in));
                        pt_map.insert("handle_out".into(), Dynamic::from(pt.handle_out));
                        pt_map.insert("weight".into(), Dynamic::from(pt.weight as f64));
                        pt_map.insert(
                            "mode".into(),
                            Dynamic::from(match pt.mode {
                                crate::cunning_core::traits::parameter::PointMode::Corner => {
                                    "Corner".to_string()
                                }
                                crate::cunning_core::traits::parameter::PointMode::Bezier => {
                                    "Bezier".to_string()
                                }
                            }),
                        );
                        points_array.push(Dynamic::from(pt_map));
                    }

                    curve_map.insert("points".into(), Dynamic::from(points_array));
                    curve_map.insert("is_closed".into(), Dynamic::from(data.is_closed));
                    let curve_type_str = match data.curve_type {
                        CurveType::Polygon => "Polygon",
                        CurveType::Bezier => "Bezier",
                        CurveType::Nurbs => "Nurbs",
                    };
                    curve_map.insert(
                        "curve_type".into(),
                        Dynamic::from(curve_type_str.to_string()),
                    );

                    Dynamic::from(curve_map)
                }
                _ => Dynamic::from(()),
            };
            params_map.insert(p.name.clone().into(), val);
        }
        scope.push("params", params_map.clone());

        // 3. Execute Script: call Rhai `main(geo, inputs, params)` and handle result robustly
        println!("RhaiNodeOp::compute: Executing script main()...");
        let call_result = engine_lock.call_fn::<Dynamic>(
            &mut scope,
            &self.ast,
            "main",
            (
                primary_geo.clone(),
                inputs_array.clone(),
                params_map.clone(),
            ),
        );

        match call_result {
            Ok(value) => {
                if let Some(result_geo) = value.clone().try_cast::<Geometry>() {
                    println!(
                        "RhaiNodeOp::compute: Success. Output Points: {}",
                        result_geo.get_point_count()
                    );
                    Arc::new(result_geo)
                } else if let Some(modified_geo) = scope.get_value::<Geometry>("geo") {
                    println!(
                        "RhaiNodeOp::compute: main returned non-Geometry, using geo from scope. Points: {}",
                        modified_geo.get_point_count()
                    );
                    Arc::new(modified_geo)
                } else {
                    println!("RhaiNodeOp::compute: main returned non-Geometry and no geo in scope, falling back to input.");
                    Arc::new(primary_geo)
                }
            }
            Err(e) => {
                println!("RhaiNodeOp::compute: Script Error: {}", e);
                error!("Rhai Script Error in {}: {}", self.script_name, e);
                if let Some(modified_geo) = scope.get_value::<Geometry>("geo") {
                    println!(
                        "RhaiNodeOp::compute: Returning modified_geo from scope after error. Points: {}",
                        modified_geo.get_point_count()
                    );
                    Arc::new(modified_geo)
                } else {
                    println!(
                        "RhaiNodeOp::compute: Error and no geo in scope, falling back to input."
                    );
                    Arc::new(primary_geo)
                }
            }
        }
    }
}

use crate::cunning_core::scripting::api::{
    GizmoScriptCommand, GizmoScriptContext, HudContext, InputContext,
};

impl NodeInteraction for RhaiNodeOp {
    fn has_hud(&self) -> bool {
        self.ui_ast.is_some()
    }
    fn draw_hud(
        &self,
        ui: &mut bevy_egui::egui::Ui,
        services: &dyn ServiceProvider,
        node_id: Uuid,
    ) {
        if let Some(ui_ast) = &self.ui_ast {
            let engine_lock = match self.engine.0.lock() {
                Ok(guard) => guard,
                Err(e) => {
                    ui.label(
                        bevy_egui::egui::RichText::new("Engine Poisoned")
                            .color(bevy_egui::egui::Color32::RED),
                    );
                    println!("Rhai Engine Lock Poisoned in HUD: {}", e);
                    return;
                }
            };

            let mut scope = Scope::new();

            let ctx = HudContext::new();
            scope.push("ctx", ctx.clone());

            // 2. Prepare Params (REAL params from ECS)
            let mut params_map = rhai::Map::new();

            if let Some(node_graph_res) = services.get::<NodeGraphResource>() {
                let graph = &node_graph_res.0;
                if let Some(node) = graph.nodes.get(&node_id) {
                    for p in &node.parameters {
                        use crate::nodes::parameter::{CurveType, ParameterValue};
                        let val = match &p.value {
                            ParameterValue::Float(f) => Dynamic::from(*f as f64),
                            ParameterValue::Int(i) => Dynamic::from(*i),
                            ParameterValue::String(s) => Dynamic::from(s.clone()),
                            ParameterValue::Bool(b) => Dynamic::from(*b),
                            ParameterValue::Vec3(v) => Dynamic::from(*v),
                            ParameterValue::Curve(data) => {
                                let mut curve_map = rhai::Map::new();

                                let mut points_array = rhai::Array::new();
                                for pt in &data.points {
                                    let mut pt_map = rhai::Map::new();
                                    pt_map.insert("id".into(), Dynamic::from(pt.id.to_string()));
                                    pt_map.insert("position".into(), Dynamic::from(pt.position));
                                    pt_map.insert("handle_in".into(), Dynamic::from(pt.handle_in));
                                    pt_map.insert("handle_out".into(), Dynamic::from(pt.handle_out));
                                    pt_map.insert("weight".into(), Dynamic::from(pt.weight as f64));
                                    pt_map.insert(
                                        "mode".into(),
                                        Dynamic::from(match pt.mode {
                                            crate::cunning_core::traits::parameter::PointMode::Corner => {
                                                "Corner".to_string()
                                            }
                                            crate::cunning_core::traits::parameter::PointMode::Bezier => {
                                                "Bezier".to_string()
                                            }
                                        }),
                                    );
                                    points_array.push(Dynamic::from(pt_map));
                                }

                                curve_map.insert("points".into(), Dynamic::from(points_array));
                                curve_map.insert("is_closed".into(), Dynamic::from(data.is_closed));
                                let curve_type_str = match data.curve_type {
                                    CurveType::Polygon => "Polygon",
                                    CurveType::Bezier => "Bezier",
                                    CurveType::Nurbs => "Nurbs",
                                };
                                curve_map.insert(
                                    "curve_type".into(),
                                    Dynamic::from(curve_type_str.to_string()),
                                );

                                Dynamic::from(curve_map)
                            }
                            _ => Dynamic::from(()),
                        };
                        params_map.insert(p.name.clone().into(), val);
                    }
                }
            }

            scope.push("params", params_map);

            let mut state_map = rhai::Map::new();
            if let Some(script_state) = services.get::<ScriptNodeState>() {
                if let Ok(per_node) = script_state.per_node.lock() {
                    if let Some(existing) = per_node.get(&node_id) {
                        state_map = existing.clone();
                    }
                }
            }
            scope.push("state", state_map);

            // 3. Call "draw_hud(ctx, params)"
            // Step A: Run AST to define functions in scope
            if let Err(e) = engine_lock.eval_ast_with_scope::<()>(&mut scope, ui_ast) {
                ui.label(
                    bevy_egui::egui::RichText::new(format!("UI Script Error: {}", e))
                        .color(bevy_egui::egui::Color32::RED),
                );
                return;
            }

            // Step B: Call "draw_hud"
            let params_val = scope.get_value::<rhai::Map>("params").unwrap_or_default();

            match engine_lock.call_fn::<()>(
                &mut scope,
                ui_ast,
                "draw_hud",
                (ctx.clone(), params_val),
            ) {
                Ok(_) => {
                    if let Ok(cmds) = ctx.commands.lock() {
                        for (cmd_type, content) in cmds.iter() {
                            match cmd_type.as_str() {
                                "heading" => {
                                    ui.heading(content);
                                }
                                "label" => {
                                    ui.label(content);
                                }
                                _ => {
                                    ui.label(format!("Unknown command: {}", content));
                                }
                            }
                        }
                    }

                    let param_updates = if let Ok(mut ops) = ctx.param_updates.lock() {
                        let collected = ops.clone();
                        ops.clear();
                        collected
                    } else {
                        Vec::new()
                    };

                    if !param_updates.is_empty() {
                        let mut updates: Vec<(String, ParamUpdateValue)> = Vec::new();
                        for (name, value) in param_updates {
                            if let Some(v) = dynamic_to_param_update_value(&value) {
                                updates.push((name, v));
                            }
                        }
                        if !updates.is_empty() {
                            let _ = enqueue_graph_command(Box::new(move |g| {
                                let Some(node) = g.nodes.get_mut(&node_id) else {
                                    return GraphCommandEffect::default();
                                };
                                use crate::nodes::parameter::ParameterValue;
                                let mut changed = false;
                                for (name, v) in &updates {
                                    let Some(p) = node.parameters.iter_mut().find(|p| p.name == *name) else {
                                        continue;
                                    };
                                    match (&mut p.value, v) {
                                        (ParameterValue::Float(dst), ParamUpdateValue::Float(x)) => {
                                            *dst = *x;
                                            changed = true;
                                        }
                                        (ParameterValue::Int(dst), ParamUpdateValue::Int(x)) => {
                                            *dst = *x;
                                            changed = true;
                                        }
                                        (ParameterValue::Bool(dst), ParamUpdateValue::Bool(x)) => {
                                            *dst = *x;
                                            changed = true;
                                        }
                                        (ParameterValue::Vec3(dst), ParamUpdateValue::Vec3(x)) => {
                                            *dst = *x;
                                            changed = true;
                                        }
                                        (ParameterValue::String(dst), ParamUpdateValue::String(x)) => {
                                            *dst = x.clone();
                                            changed = true;
                                        }
                                        _ => {}
                                    }
                                }
                                if changed {
                                    g.mark_dirty(node_id);
                                    GraphCommandEffect {
                                        graph_changed: true,
                                        geometry_changed: true,
                                    }
                                } else {
                                    GraphCommandEffect::default()
                                }
                            }));
                        }
                    }

                    let state_updates = if let Ok(mut ops) = ctx.state_updates.lock() {
                        let collected = ops.clone();
                        ops.clear();
                        collected
                    } else {
                        Vec::new()
                    };

                    if !state_updates.is_empty() {
                        if let Some(script_state) = services.get::<ScriptNodeState>() {
                            if let Ok(mut per_node) = script_state.per_node.lock() {
                                let entry = per_node.entry(node_id).or_insert_with(rhai::Map::new);
                                for (key, value) in state_updates {
                                    entry.insert(key.into(), value);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    ui.label(
                        bevy_egui::egui::RichText::new(format!("DrawHUD Call Error: {}", e))
                            .color(bevy_egui::egui::Color32::RED),
                    );
                    println!("DrawHUD Call Error: {}", e);
                }
            }
        }
    }

    fn draw_gizmos(
        &self,
        buffer: &mut GizmoDrawBuffer,
        context: &GizmoContext,
        _gizmo_state: &mut GizmoState,
        services: &dyn ServiceProvider,
        node_id: Uuid,
    ) {
        if let Some(ui_ast) = &self.ui_ast {
            let engine_lock = match self.engine.0.lock() {
                Ok(guard) => guard,
                Err(e) => {
                    println!("Rhai Engine Lock Poisoned in Gizmos: {}", e);
                    return;
                }
            };

            let mut scope = Scope::new();

            let ctx = HudContext::new();
            scope.push("ctx", ctx.clone());

            // 1. Params
            let mut params_map = rhai::Map::new();
            if let Some(node_graph_res) = services.get::<NodeGraphResource>() {
                let graph = &node_graph_res.0;
                if let Some(node) = graph.nodes.get(&node_id) {
                    for p in &node.parameters {
                        use crate::nodes::parameter::{CurveType, ParameterValue};
                        let val = match &p.value {
                            ParameterValue::Float(f) => Dynamic::from(*f as f64),
                            ParameterValue::Int(i) => Dynamic::from(*i),
                            ParameterValue::String(s) => Dynamic::from(s.clone()),
                            ParameterValue::Bool(b) => Dynamic::from(*b),
                            ParameterValue::Vec3(v) => Dynamic::from(*v),
                            ParameterValue::Curve(data) => {
                                let mut curve_map = rhai::Map::new();
                                let mut points_array = rhai::Array::new();
                                for pt in &data.points {
                                    let mut pt_map = rhai::Map::new();
                                    pt_map.insert("id".into(), Dynamic::from(pt.id.to_string()));
                                    pt_map.insert("position".into(), Dynamic::from(pt.position));
                                    pt_map.insert("handle_in".into(), Dynamic::from(pt.handle_in));
                                    pt_map.insert("handle_out".into(), Dynamic::from(pt.handle_out));
                                    pt_map.insert("weight".into(), Dynamic::from(pt.weight as f64));
                                    pt_map.insert(
                                        "mode".into(),
                                        Dynamic::from(match pt.mode {
                                            crate::cunning_core::traits::parameter::PointMode::Corner => {
                                                "Corner".to_string()
                                            }
                                            crate::cunning_core::traits::parameter::PointMode::Bezier => {
                                                "Bezier".to_string()
                                            }
                                        }),
                                    );
                                    points_array.push(Dynamic::from(pt_map));
                                }
                                curve_map.insert("points".into(), Dynamic::from(points_array));
                                curve_map.insert("is_closed".into(), Dynamic::from(data.is_closed));
                                let curve_type_str = match data.curve_type {
                                    CurveType::Polygon => "Polygon",
                                    CurveType::Bezier => "Bezier",
                                    CurveType::Nurbs => "Nurbs",
                                };
                                curve_map.insert(
                                    "curve_type".into(),
                                    Dynamic::from(curve_type_str.to_string()),
                                );
                                Dynamic::from(curve_map)
                            }
                            _ => Dynamic::from(()),
                        };
                        params_map.insert(p.name.clone().into(), val);
                    }
                }
            }
            scope.push("params", params_map);

            // 2. State
            let mut state_map = rhai::Map::new();
            if let Some(script_state) = services.get::<ScriptNodeState>() {
                if let Ok(per_node) = script_state.per_node.lock() {
                    if let Some(existing) = per_node.get(&node_id) {
                        state_map = existing.clone();
                    }
                }
            }
            scope.push("state", state_map);

            // 3. Input / Gizmo context
            let input_ctx = InputContext {
                ray_origin: context.ray_origin,
                ray_direction: context.ray_direction,
                cursor_pos: context.cursor_pos,
                mouse_left_pressed: context.mouse_left_pressed,
                mouse_left_just_pressed: context.mouse_left_just_pressed,
                mouse_left_just_released: context.mouse_left_just_released,
                cam_pos: context.cam_pos,
                is_orthographic: context.is_orthographic,
                scale_factor: context.scale_factor,
            };
            scope.push("input", input_ctx.clone());

            let gizmo_ctx = GizmoScriptContext::new();
            scope.push("gizmo", gizmo_ctx.clone());

            // 4. Eval UI AST to register functions
            if let Err(e) = engine_lock.eval_ast_with_scope::<()>(&mut scope, ui_ast) {
                println!("Gizmo Script Error: {}", e);
                return;
            }

            // 5. Call optional draw_gizmos if defined
            let params_val = scope.get_value::<rhai::Map>("params").unwrap_or_default();
            let result = engine_lock.call_fn::<()>(
                &mut scope,
                ui_ast,
                "draw_gizmos",
                (gizmo_ctx.clone(), params_val, input_ctx, ctx.clone()),
            );

            if let Err(e) = result {
                // It's acceptable that some scripts don't define draw_gizmos; ignore FnNotFound
                println!("draw_gizmos call error: {}", e);
                return;
            }

            // 6. Apply param updates from gizmo via ctx
            let param_updates = if let Ok(mut ops) = ctx.param_updates.lock() {
                let collected = ops.clone();
                ops.clear();
                collected
            } else {
                Vec::new()
            };

            if !param_updates.is_empty() {
                let mut updates: Vec<(String, ParamUpdateValue)> = Vec::new();
                for (name, value) in param_updates {
                    if let Some(v) = dynamic_to_param_update_value(&value) {
                        updates.push((name, v));
                    }
                }
                if !updates.is_empty() {
                    let _ = enqueue_graph_command(Box::new(move |g| {
                        let Some(node) = g.nodes.get_mut(&node_id) else {
                            return GraphCommandEffect::default();
                        };
                        use crate::nodes::parameter::ParameterValue;
                        let mut changed = false;
                        for (name, v) in &updates {
                            let Some(p) = node.parameters.iter_mut().find(|p| p.name == *name) else {
                                continue;
                            };
                            match (&mut p.value, v) {
                                (ParameterValue::Float(dst), ParamUpdateValue::Float(x)) => {
                                    *dst = *x;
                                    changed = true;
                                }
                                (ParameterValue::Int(dst), ParamUpdateValue::Int(x)) => {
                                    *dst = *x;
                                    changed = true;
                                }
                                (ParameterValue::Bool(dst), ParamUpdateValue::Bool(x)) => {
                                    *dst = *x;
                                    changed = true;
                                }
                                (ParameterValue::Vec3(dst), ParamUpdateValue::Vec3(x)) => {
                                    *dst = *x;
                                    changed = true;
                                }
                                (ParameterValue::String(dst), ParamUpdateValue::String(x)) => {
                                    *dst = x.clone();
                                    changed = true;
                                }
                                _ => {}
                            }
                        }
                        if changed {
                            g.mark_dirty(node_id);
                            GraphCommandEffect {
                                graph_changed: true,
                                geometry_changed: true,
                            }
                        } else {
                            GraphCommandEffect::default()
                        }
                    }));
                }
            }

            // 7. Apply state updates (from `state` map in scope)
            if let Some(script_state) = services.get::<ScriptNodeState>() {
                if let Ok(mut per_node) = script_state.per_node.lock() {
                    let entry = per_node.entry(node_id).or_insert_with(rhai::Map::new);
                    if let Some(new_state) = scope.get_value::<rhai::Map>("state") {
                        *entry = new_state;
                    }
                }
            }

            // 8. Translate Gizmo commands
            let cmds_result = gizmo_ctx.commands.lock();
            if let Ok(cmds) = cmds_result {
                for cmd in cmds.iter() {
                    match cmd {
                        GizmoScriptCommand::Line { start, end, color } => {
                            buffer.draw_line(*start, *end, Color::srgb(color.x, color.y, color.z));
                        }
                        GizmoScriptCommand::Sphere {
                            center,
                            radius,
                            color,
                        } => {
                            let transform = Transform::from_translation(*center)
                                .with_scale(Vec3::splat(*radius));
                            buffer.draw_mesh(
                                GizmoPrimitive::Sphere,
                                transform,
                                Color::srgb(color.x, color.y, color.z),
                            );
                        }
                        GizmoScriptCommand::Cube {
                            center,
                            size,
                            color,
                        } => {
                            let transform =
                                Transform::from_translation(*center).with_scale(Vec3::splat(*size));
                            buffer.draw_mesh(
                                GizmoPrimitive::Cube,
                                transform,
                                Color::srgb(color.x, color.y, color.z),
                            );
                        }
                    }
                }
            }
        }
    }
}
