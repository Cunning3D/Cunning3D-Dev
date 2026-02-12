use std::path::Path;

fn is_plugin_intent(s: &str) -> bool {
    let t = s.to_ascii_lowercase();
    // Keep auto-pack strongly plugin-oriented to avoid biasing normal "create node" chat into Rust code writing.
    let strong = [
        "nodespec", "hostapi", "abi", "hud", "gizmo", "plugin", "dll", "cdylib", "pick_id", "vtable", "callback", "热加载", "插件",
        "rust节点", "插件节点",
    ];
    let rustish = ["rust", "wgsl", "shader", "compute", "gpu"];
    let interaction = ["viewport", "handle", "drag", "click", "keyboard", "shortcut", "interaction", "draggable", "clickable", "拖拽", "点击", "快捷键", "交互"];
    strong.iter().any(|k| t.contains(k) || s.contains(k))
        || (rustish.iter().any(|k| t.contains(k) || s.contains(k))
            && interaction.iter().any(|k| t.contains(k) || s.contains(k)))
}

fn excerpt_file(base: &Path, rel: &str, lang: &str, patterns: &[&str], max_lines: usize) -> String {
    let p = base.join(rel);
    let Ok(raw) = std::fs::read_to_string(&p) else {
        return String::new();
    };
    let lines: Vec<&str> = raw.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    let mut start = 0usize;
    if let Some((idx, _)) = lines
        .iter()
        .enumerate()
        .find(|(_, l)| patterns.iter().any(|pat| l.contains(pat)))
    {
        start = idx.saturating_sub(max_lines / 3);
    }
    let end = (start + max_lines).min(lines.len());
    let body = lines[start..end].join("\n");
    format!("\n// @file: {rel}\n```{lang}\n{body}\n```\n")
}

/// Auto-inject a small, curated context pack for plugin nodes (HostApi/HUD/Gizmo/interaction).
pub fn collect_auto_pack(user_text: &str, base_dir: &Path) -> String {
    if !is_plugin_intent(user_text) {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("\n[Auto Context Pack: Plugin Interaction ABI]\n");
    out.push_str(&excerpt_file(
        base_dir,
        "src/cunning_core/plugin_system/c_api.rs",
        "rs",
        &[
            "CHostApi",
            "CHudCmd",
            "CGizmoCmd",
            "CInputEvent",
            "CNodeInteractionVTable",
        ],
        160,
    ));
    out.push_str(&excerpt_file(
        base_dir,
        "src/cunning_core/plugin_system/mod.rs",
        "rs",
        &[
            "node_state_get",
            "node_state_set",
            "node_curve_get",
            "node_curve_set",
            "plugin_gizmo_build",
            "plugin_input_event_mouse",
        ],
        160,
    ));
    out.push_str(&excerpt_file(
        base_dir,
        "plugins/extra_node/curve_plugin/src/lib.rs",
        "rs",
        &[
            "hud_build",
            "hud_event",
            "gizmo_build",
            "gizmo_event",
            "input_event_mouse",
        ],
        160,
    ));
    out.push_str(&excerpt_file(
        base_dir,
        "src/gizmos/input.rs",
        "rs",
        &[
            "PluginPick",
            "plugin_input_event",
            "gizmo_event_drag",
            "gizmo_event_click",
        ],
        120,
    ));
    out.push_str(&excerpt_file(
        base_dir,
        "src/gizmos/plugin_gizmos.rs",
        "rs",
        &["sync_plugin_gizmos", "PluginPick"],
        120,
    ));
    out
}
