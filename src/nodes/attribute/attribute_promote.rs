//! `attribute_promote` node logic.

use crate::libs::geometry::geo_ref::GeometryRef;
use crate::{
    cunning_core::traits::node_interface::{NodeOp, NodeParameters},
    libs::algorithms::algorithms_dcc::PagedBuffer,
    libs::geometry::ids::{PointId, VertexId},
    mesh::{Attribute, Geometry},
    nodes::{
        parameter::{Parameter, ParameterUIType, ParameterValue},
        GeoLevel, InputStyle, NodeStyle,
    },
    register_node,
};
use bevy::math::{DVec2, DVec3, DVec4};
use bevy::prelude::{Vec2, Vec3, Vec4};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct AttributePromoteNode;

impl NodeParameters for AttributePromoteNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "name",
                "Name",
                "Settings",
                ParameterValue::String("@Cd".to_string()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "source_level",
                "Source Level",
                "Settings",
                ParameterValue::Int(GeoLevel::Point as i32),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Point".to_string(), GeoLevel::Point as i32),
                        ("Vertex".to_string(), GeoLevel::Vertex as i32),
                    ],
                },
            ),
            Parameter::new(
                "dest_level",
                "Destination Level",
                "Settings",
                ParameterValue::Int(GeoLevel::Vertex as i32),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Point".to_string(), GeoLevel::Point as i32),
                        ("Vertex".to_string(), GeoLevel::Vertex as i32),
                        ("Primitive".to_string(), GeoLevel::Primitive as i32),
                    ],
                },
            ),
            Parameter::new(
                "method",
                "Method",
                "Settings",
                ParameterValue::Int(AggregationMethod::Average as i32),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Average".to_string(), AggregationMethod::Average as i32),
                        ("Sum".to_string(), AggregationMethod::Sum as i32),
                        ("Min".to_string(), AggregationMethod::Min as i32),
                        ("Max".to_string(), AggregationMethod::Max as i32),
                    ],
                },
            ),
            Parameter::new(
                "keep_original",
                "Keep Original",
                "Settings",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for AttributePromoteNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let default_input = Arc::new(Geometry::new());
        let input = mats.first().unwrap_or(&default_input);
        let param_map: HashMap<String, ParameterValue> = params
            .iter()
            .map(|p| (p.name.clone(), p.value.clone()))
            .collect();
        match promote(input, &param_map) {
            Ok(geo) => Arc::new(geo),
            Err(_) => input.clone(),
        }
    }
}

register_node!("Attribute Promote", "Attribute", AttributePromoteNode);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AttributePromoteParams {
    pub name: String,
    pub from_class: GeoLevel,
    pub to_class: GeoLevel,
    pub method: AggregationMethod,
    pub keep_original: bool,
}

impl Default for AttributePromoteParams {
    fn default() -> Self {
        Self {
            name: "".to_string(),
            from_class: GeoLevel::Point,
            to_class: GeoLevel::Vertex,
            method: AggregationMethod::Average,
            keep_original: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AggregationMethod {
    Average,
    Sum,
    Min,
    Max,
}

/// Unified Accessor for Attribute data (Vec or PagedBuffer)
/// Includes T: Clone + Send + Sync + 'static bound to satisfy PagedBuffer requirement
enum AttrAccessor<'a, T: Clone + Send + Sync + 'static> {
    Slice(&'a [T]),
    Paged(&'a PagedBuffer<T>),
}

impl<'a, T: Clone + Send + Sync + 'static> AttrAccessor<'a, T> {
    fn new(attr: &'a Attribute) -> Option<Self> {
        if let Some(slice) = attr.as_slice::<T>() {
            Some(Self::Slice(slice))
        } else if let Some(paged) = attr.as_paged::<T>() {
            Some(Self::Paged(paged))
        } else {
            None
        }
    }

    #[inline(always)]
    fn get(&self, index: usize) -> Option<T> {
        match self {
            Self::Slice(s) => s.get(index).cloned(),
            Self::Paged(p) => p.get(index),
        }
    }
}

pub fn promote(
    input_geo: &Geometry,
    parameters: &HashMap<String, ParameterValue>,
) -> Result<Geometry, String> {
    let params = parse_promote_params(parameters);
    promote_with_params(input_geo, &params)
}

pub fn parse_promote_params(
    parameters: &HashMap<String, ParameterValue>,
) -> AttributePromoteParams {
    let name = match parameters.get("name") {
        Some(ParameterValue::String(s)) => s.clone(),
        _ => "@Cd".to_string(),
    };
    let source_level = match parameters.get("source_level") {
        Some(ParameterValue::Int(i)) => GeoLevel::from_i32(*i).unwrap_or(GeoLevel::Point),
        _ => GeoLevel::Point,
    };
    let dest_level = match parameters.get("dest_level") {
        Some(ParameterValue::Int(i)) => GeoLevel::from_i32(*i).unwrap_or(GeoLevel::Vertex),
        _ => GeoLevel::Vertex,
    };
    let method = match parameters.get("method") {
        Some(ParameterValue::Int(i)) => {
            let i = *i;
            if i == AggregationMethod::Average as i32 {
                AggregationMethod::Average
            } else if i == AggregationMethod::Sum as i32 {
                AggregationMethod::Sum
            } else if i == AggregationMethod::Min as i32 {
                AggregationMethod::Min
            } else if i == AggregationMethod::Max as i32 {
                AggregationMethod::Max
            } else {
                AggregationMethod::Average
            }
        }
        _ => AggregationMethod::Average,
    };
    let keep_original = match parameters.get("keep_original") {
        Some(ParameterValue::Bool(b)) => *b,
        _ => true,
    };
    AttributePromoteParams {
        name,
        from_class: source_level,
        to_class: dest_level,
        method,
        keep_original,
    }
}

pub fn promote_with_params(
    input_geo: &Geometry,
    params: &AttributePromoteParams,
) -> Result<Geometry, String> {
    let name = params.name.clone();
    let source_level = params.from_class;
    let dest_level = params.to_class;
    let method = params.method;
    let keep_original = params.keep_original;

    if name == "@P" && source_level == GeoLevel::Point {
        return Err("Cannot promote the position attribute '@P' from the Point level.".to_string());
    }

    let mut output_geo = input_geo.clone();

    match (source_level, dest_level) {
        (GeoLevel::Point, GeoLevel::Vertex) => {
            if let Some(promoted_attr) = promote_point_to_vertex(&output_geo, &name) {
                output_geo.insert_vertex_attribute(name.clone(), promoted_attr);
            }
        }
        (GeoLevel::Vertex, GeoLevel::Point) => {
            if let Some(promoted_attr) = promote_vertex_to_point(&output_geo, &name, method) {
                output_geo.insert_point_attribute(name.clone(), promoted_attr);
            }
        }
        (GeoLevel::Vertex, GeoLevel::Primitive) => {
            if let Some(promoted_attr) = promote_vertex_to_primitive(&output_geo, &name, method) {
                output_geo.insert_primitive_attribute(name.clone(), promoted_attr);
            }
        }
        _ => {}
    }

    if !keep_original {
        if source_level == GeoLevel::Point {
            output_geo.remove_point_attribute(name.as_str());
        } else if source_level == GeoLevel::Vertex {
            output_geo.remove_vertex_attribute(name.as_str());
        }
    }

    Ok(output_geo)
}

fn promote_point_to_vertex(geo: &Geometry, name: &str) -> Option<Attribute> {
    let attr = geo.get_point_attribute(name)?;

    macro_rules! match_promote {
        ($t:ty) => {
            if let Some(accessor) = AttrAccessor::<$t>::new(attr) {
                let mut new_vals = Vec::with_capacity(geo.vertices().len());
                for v in geo.vertices().values() {
                    if let Some(pt_idx) = geo.points().get_dense_index(v.point_id.into()) {
                        new_vals.push(accessor.get(pt_idx).unwrap_or_default());
                    } else {
                        new_vals.push(Default::default());
                    }
                }
                return Some(Attribute::new_auto(new_vals));
            }
        };
    }

    match_promote!(f32);
    match_promote!(Vec2);
    match_promote!(Vec3);
    match_promote!(Vec4);
    match_promote!(i32);
    match_promote!(bool);
    match_promote!(String);
    match_promote!(f64);
    match_promote!(DVec2);
    match_promote!(DVec3);
    match_promote!(DVec4);

    None
}

fn promote_vertex_to_point(
    geo: &Geometry,
    name: &str,
    method: AggregationMethod,
) -> Option<Attribute> {
    let attr = geo.get_vertex_attribute(name)?;

    let mut point_to_vertex_map: HashMap<PointId, Vec<usize>> = HashMap::new();
    for (v_dense_idx, vertex) in geo.vertices().values().iter().enumerate() {
        point_to_vertex_map
            .entry(vertex.point_id)
            .or_default()
            .push(v_dense_idx);
    }

    let num_points = geo.points().len();

    // Helper macro for dispatching aggregation
    macro_rules! promote_dispatch {
        ($t:ty, $zero:expr, $min_val:expr, $max_val:expr, $add:expr, $div:expr, $min_op:expr, $max_op:expr) => {
            if let Some(accessor) = AttrAccessor::<$t>::new(attr) {
                let mut new_attrs = vec![$zero; num_points];
                for (p_dense_idx, (p_id, _)) in geo.points().iter_enumerated().enumerate() {
                    let pid = PointId::from(p_id);
                    if let Some(v_indices) = point_to_vertex_map.get(&pid) {
                        if !v_indices.is_empty() {
                            match method {
                                AggregationMethod::Average => {
                                    let mut sum = $zero;
                                    let mut count = 0;
                                    for &idx in v_indices {
                                        if let Some(val) = accessor.get(idx) {
                                            sum = $add(sum, val);
                                            count += 1;
                                        }
                                    }
                                    if count > 0 {
                                        new_attrs[p_dense_idx] = $div(sum, count as f32);
                                    }
                                }
                                AggregationMethod::Sum => {
                                    let mut sum = $zero;
                                    for &idx in v_indices {
                                        if let Some(val) = accessor.get(idx) {
                                            sum = $add(sum, val);
                                        }
                                    }
                                    new_attrs[p_dense_idx] = sum;
                                }
                                AggregationMethod::Min => {
                                    let mut min_v = $max_val; // Init with max to find min
                                    let mut found = false;
                                    for &idx in v_indices {
                                        if let Some(val) = accessor.get(idx) {
                                            min_v = $min_op(min_v, val);
                                            found = true;
                                        }
                                    }
                                    if found {
                                        new_attrs[p_dense_idx] = min_v;
                                    }
                                }
                                AggregationMethod::Max => {
                                    let mut max_v = $min_val; // Init with min to find max
                                    let mut found = false;
                                    for &idx in v_indices {
                                        if let Some(val) = accessor.get(idx) {
                                            max_v = $max_op(max_v, val);
                                            found = true;
                                        }
                                    }
                                    if found {
                                        new_attrs[p_dense_idx] = max_v;
                                    }
                                }
                            }
                        }
                    }
                }
                return Some(Attribute::new_auto(new_attrs));
            }
        };
    }

    // Helper macro for fallback (only First)
    macro_rules! promote_first {
        ($t:ty, $zero:expr) => {
            if let Some(accessor) = AttrAccessor::<$t>::new(attr) {
                let mut new_attrs = vec![$zero; num_points];
                for (p_dense_idx, (p_id, _)) in geo.points().iter_enumerated().enumerate() {
                    let pid = PointId::from(p_id);
                    if let Some(v_indices) = point_to_vertex_map.get(&pid) {
                        if let Some(&first_idx) = v_indices.first() {
                            if let Some(val) = accessor.get(first_idx) {
                                new_attrs[p_dense_idx] = val;
                            }
                        }
                    }
                }
                return Some(Attribute::new_auto(new_attrs));
            }
        };
    }

    promote_dispatch!(
        f32,
        0.0,
        f32::MIN,
        f32::MAX,
        |a, b| a + b,
        |s: f32, c: f32| s / c,
        |a: f32, b: f32| a.min(b),
        |a: f32, b: f32| a.max(b)
    );
    promote_dispatch!(
        Vec3,
        Vec3::ZERO,
        Vec3::splat(f32::MIN),
        Vec3::splat(f32::MAX),
        |a, b| a + b,
        |s: Vec3, c: f32| s / c,
        |a: Vec3, b: Vec3| a.min(b),
        |a: Vec3, b: Vec3| a.max(b)
    );
    promote_dispatch!(
        Vec2,
        Vec2::ZERO,
        Vec2::splat(f32::MIN),
        Vec2::splat(f32::MAX),
        |a, b| a + b,
        |s: Vec2, c: f32| s / c,
        |a: Vec2, b: Vec2| a.min(b),
        |a: Vec2, b: Vec2| a.max(b)
    );
    promote_dispatch!(
        Vec4,
        Vec4::ZERO,
        Vec4::splat(f32::MIN),
        Vec4::splat(f32::MAX),
        |a, b| a + b,
        |s: Vec4, c: f32| s / c,
        |a: Vec4, b: Vec4| a.min(b),
        |a: Vec4, b: Vec4| a.max(b)
    );

    promote_first!(i32, 0);
    promote_first!(bool, false);
    promote_first!(String, String::new());

    None
}

fn promote_vertex_to_primitive(
    geo: &Geometry,
    name: &str,
    method: AggregationMethod,
) -> Option<Attribute> {
    let attr = geo.get_vertex_attribute(name)?;
    let num_prims = geo.primitives().len();

    // Helper macro for dispatching aggregation
    macro_rules! promote_prim_dispatch {
        ($t:ty, $zero:expr, $min_val:expr, $max_val:expr, $add:expr, $div:expr, $min_op:expr, $max_op:expr) => {
            if let Some(accessor) = AttrAccessor::<$t>::new(attr) {
                let mut new_attrs = Vec::with_capacity(num_prims);
                for prim in geo.primitives().values() {
                    let verts = prim.vertices();
                    if verts.is_empty() {
                        new_attrs.push($zero);
                        continue;
                    }

                    match method {
                        AggregationMethod::Average => {
                            let mut sum = $zero;
                            let mut count = 0;
                            for &vid in verts {
                                if let Some(v_idx) = geo.vertices().get_dense_index(vid.into()) {
                                    if let Some(val) = accessor.get(v_idx) {
                                        sum = $add(sum, val);
                                        count += 1;
                                    }
                                }
                            }
                            if count > 0 {
                                new_attrs.push($div(sum, count as f32));
                            } else {
                                new_attrs.push($zero);
                            }
                        }
                        AggregationMethod::Sum => {
                            let mut sum = $zero;
                            for &vid in verts {
                                if let Some(v_idx) = geo.vertices().get_dense_index(vid.into()) {
                                    if let Some(val) = accessor.get(v_idx) {
                                        sum = $add(sum, val);
                                    }
                                }
                            }
                            new_attrs.push(sum);
                        }
                        AggregationMethod::Min => {
                            let mut min_v = $max_val;
                            let mut found = false;
                            for &vid in verts {
                                if let Some(v_idx) = geo.vertices().get_dense_index(vid.into()) {
                                    if let Some(val) = accessor.get(v_idx) {
                                        min_v = $min_op(min_v, val);
                                        found = true;
                                    }
                                }
                            }
                            if found {
                                new_attrs.push(min_v);
                            } else {
                                new_attrs.push($zero);
                            }
                        }
                        AggregationMethod::Max => {
                            let mut max_v = $min_val;
                            let mut found = false;
                            for &vid in verts {
                                if let Some(v_idx) = geo.vertices().get_dense_index(vid.into()) {
                                    if let Some(val) = accessor.get(v_idx) {
                                        max_v = $max_op(max_v, val);
                                        found = true;
                                    }
                                }
                            }
                            if found {
                                new_attrs.push(max_v);
                            } else {
                                new_attrs.push($zero);
                            }
                        }
                    }
                }
                return Some(Attribute::new_auto(new_attrs));
            }
        };
    }

    // Helper for first
    macro_rules! promote_prim_first {
        ($t:ty, $zero:expr) => {
            if let Some(accessor) = AttrAccessor::<$t>::new(attr) {
                let mut new_attrs = Vec::with_capacity(num_prims);
                for prim in geo.primitives().values() {
                    let verts = prim.vertices();
                    let mut found = false;
                    if let Some(&first_vid) = verts.first() {
                        if let Some(v_idx) = geo.vertices().get_dense_index(first_vid.into()) {
                            if let Some(val) = accessor.get(v_idx) {
                                new_attrs.push(val);
                                found = true;
                            }
                        }
                    }
                    if !found {
                        new_attrs.push($zero);
                    }
                }
                return Some(Attribute::new_auto(new_attrs));
            }
        };
    }

    promote_prim_dispatch!(
        f32,
        0.0,
        f32::MIN,
        f32::MAX,
        |a, b| a + b,
        |s: f32, c: f32| s / c,
        |a: f32, b: f32| a.min(b),
        |a: f32, b: f32| a.max(b)
    );
    promote_prim_dispatch!(
        Vec3,
        Vec3::ZERO,
        Vec3::splat(f32::MIN),
        Vec3::splat(f32::MAX),
        |a, b| a + b,
        |s: Vec3, c: f32| s / c,
        |a: Vec3, b: Vec3| a.min(b),
        |a: Vec3, b: Vec3| a.max(b)
    );
    promote_prim_dispatch!(
        Vec2,
        Vec2::ZERO,
        Vec2::splat(f32::MIN),
        Vec2::splat(f32::MAX),
        |a, b| a + b,
        |s: Vec2, c: f32| s / c,
        |a: Vec2, b: Vec2| a.min(b),
        |a: Vec2, b: Vec2| a.max(b)
    );
    promote_prim_dispatch!(
        Vec4,
        Vec4::ZERO,
        Vec4::splat(f32::MIN),
        Vec4::splat(f32::MAX),
        |a, b| a + b,
        |s: Vec4, c: f32| s / c,
        |a: Vec4, b: Vec4| a.min(b),
        |a: Vec4, b: Vec4| a.max(b)
    );

    promote_prim_first!(i32, 0);
    promote_prim_first!(bool, false);
    promote_prim_first!(String, String::new());

    None
}

pub fn node_style() -> NodeStyle {
    NodeStyle::Normal
}

pub fn input_style() -> InputStyle {
    InputStyle::Individual
}
