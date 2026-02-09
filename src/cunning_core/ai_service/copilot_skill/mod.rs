//! Copilot Skill system - lightweight read-only tools for deep thinking mode
use super::native_tiny_model::knowledge_cache::{NodeKnowledge, GLOBAL_KNOWLEDGE_CACHE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDef {
    pub name: &'static str,
    pub description: &'static str,
    pub params: &'static str,
}

pub const SKILL_LIST: &[SkillDef] = &[
    SkillDef {
        name: "list_nodes",
        description: "List node names, optionally filtered by category or keyword",
        params: "category?: string, keyword?: string",
    },
    SkillDef {
        name: "get_node",
        description: "Get full knowledge of a single node",
        params: "name: string",
    },
    SkillDef {
        name: "get_wrangle_syntax",
        description: "Get VEX/Rhai wrangle syntax docs",
        params: "",
    },
    SkillDef {
        name: "get_type_hierarchy",
        description: "Get data type inheritance tree",
        params: "",
    },
    SkillDef {
        name: "get_patterns",
        description: "Get common node chain patterns (few-shot)",
        params: "goal?: string",
    },
];

pub const MAX_SKILL_TURNS: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCall {
    pub skill: String,
    pub args: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub enum SkillResult {
    Text(String),
    Json(Value),
}

pub fn execute_skill(call: &SkillCall) -> SkillResult {
    match call.skill.as_str() {
        "list_nodes" => exec_list_nodes(&call.args),
        "get_node" => exec_get_node(&call.args),
        "get_wrangle_syntax" => exec_get_wrangle_syntax(),
        "get_type_hierarchy" => exec_get_type_hierarchy(),
        "get_patterns" => exec_get_patterns(&call.args),
        _ => SkillResult::Text(format!("Unknown skill: {}", call.skill)),
    }
}

fn exec_list_nodes(args: &HashMap<String, Value>) -> SkillResult {
    let category = args.get("category").and_then(|v| v.as_str());
    let keyword = args
        .get("keyword")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase());
    let Some(cache) = GLOBAL_KNOWLEDGE_CACHE.get() else {
        return SkillResult::Text("Knowledge not loaded".into());
    };
    let mut names: Vec<&str> = cache
        .nodes
        .iter()
        .filter(|(name, k)| {
            if let Some(cat) = category {
                if !k.category.eq_ignore_ascii_case(cat) {
                    return false;
                }
            }
            if let Some(ref kw) = keyword {
                let text = format!("{} {} {}", name, k.category, k.description).to_lowercase();
                if !text.contains(kw) {
                    return false;
                }
            }
            true
        })
        .map(|(n, _)| n.as_str())
        .collect();
    names.sort();
    if names.len() > 100 {
        names.truncate(100);
    }
    SkillResult::Json(serde_json::json!(names))
}

fn exec_get_node(args: &HashMap<String, Value>) -> SkillResult {
    let Some(name) = args.get("name").and_then(|v| v.as_str()) else {
        return SkillResult::Text("Missing 'name' arg".into());
    };
    let Some(cache) = GLOBAL_KNOWLEDGE_CACHE.get() else {
        return SkillResult::Text("Knowledge not loaded".into());
    };
    let Some(k) = cache.nodes.get(name) else {
        return SkillResult::Text(format!("Node '{}' not found", name));
    };
    let editable_params: HashMap<&str, &str> = k
        .parameters
        .iter()
        .filter(|(_, v)| !is_numeric_type(v))
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    SkillResult::Json(serde_json::json!({
        "name": k.name, "category": k.category, "description": k.description,
        "io": { "input_type": k.io.input_type, "output_type": k.io.output_type,
            "inputs": k.io.inputs.iter().map(|p| serde_json::json!({"name": p.name, "desc": p.description})).collect::<Vec<_>>(),
            "outputs": k.io.outputs.iter().map(|p| serde_json::json!({"name": p.name, "desc": p.description})).collect::<Vec<_>>(),
        },
        "editable_params": editable_params,
        "usage": k.usage.as_ref().map(|u| &u.text),
    }))
}

fn exec_get_wrangle_syntax() -> SkillResult {
    SkillResult::Text(
        r#"[Wrangle Syntax (Rhai)]
- `@P`, `@N`, `@Cd`, `@uv` are point attributes
- `set_attrib("name", value)` to write
- `point_count()`, `prim_count()` for counts
- `lerp(a, b, t)`, `fit(v, omin, omax, nmin, nmax)`
- Loop: `for i in 0..n { ... }`
"#
        .into(),
    )
}

fn exec_get_type_hierarchy() -> SkillResult {
    SkillResult::Text(
        r#"[Type Hierarchy]
- Geometry (mesh, points, prims)
  - Mesh (triangles, quads, polygons)
  - Curve (polyline, bezier, nurbs)
  - Points (scatter, particle)
- Sdf (signed distance field)
- Volume (voxel grid)
- Image (texture, 2D)
"#
        .into(),
    )
}

fn exec_get_patterns(args: &HashMap<String, Value>) -> SkillResult {
    let goal = args.get("goal").and_then(|v| v.as_str()).unwrap_or("");
    let mut patterns = vec![
        (
            "box_with_bevel",
            vec!["Create Box", "Bevel"],
            "Create a box with beveled edges",
        ),
        (
            "scatter_on_surface",
            vec!["Create Sphere", "Scatter"],
            "Scatter points on geometry surface",
        ),
        (
            "boolean_subtract",
            vec!["Create Box", "Create Sphere", "Boolean"],
            "Subtract sphere from box",
        ),
        (
            "extrude_poly",
            vec!["Create Grid", "PolyExtrude"],
            "Extrude polygons from grid",
        ),
    ];
    let goal_lower = goal.to_lowercase();
    if !goal_lower.is_empty() {
        patterns.retain(|(name, _, desc)| {
            name.to_lowercase().contains(&goal_lower) || desc.to_lowercase().contains(&goal_lower)
        });
    }
    SkillResult::Json(serde_json::json!(patterns
        .iter()
        .map(|(n, nodes, d)| serde_json::json!({"name": n, "nodes": nodes, "description": d}))
        .collect::<Vec<_>>()))
}

fn is_numeric_type(ty: &str) -> bool {
    let t = ty.to_lowercase();
    t.contains("f32")
        || t.contains("f64")
        || t.contains("float")
        || t.contains("i32")
        || t.contains("int")
        || t.contains("vec2")
        || t.contains("vec3")
        || t.contains("vec4")
}

pub fn build_skill_list_prompt() -> String {
    let mut s = String::from("[Available Skills]\n");
    for sk in SKILL_LIST {
        s.push_str(&format!(
            "- {}({}) : {}\n",
            sk.name, sk.params, sk.description
        ));
    }
    s.push_str("\nCall format: {\"skill\": \"name\", \"args\": {...}}\n");
    s
}

pub fn build_deep_system_prompt() -> String {
    format!(
        r#"You are Cunning3D Copilot (Deep Mode).
You plan node chains by calling skills to gather info, then output final JSON.

## Rules
1. Call skills to get necessary info before outputting final result
2. Only adjust mode/enum/bool/string params, NEVER numeric values
3. Max {} skill calls
4. Final output format: {{"nodes": [...], "params": {{"NodeName.param": "value"}}, "reason": "..."}}

{}
"#,
        MAX_SKILL_TURNS,
        build_skill_list_prompt()
    )
}

pub fn try_parse_skill_call(text: &str) -> Option<SkillCall> {
    let trimmed = text.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    let v: Value = serde_json::from_str(trimmed).ok()?;
    if v.get("skill").is_some() {
        serde_json::from_value(v).ok()
    } else {
        None
    }
}
