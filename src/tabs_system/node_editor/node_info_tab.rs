use crate::tabs_system::node_editor::cda;
use crate::{
    mesh::Geometry,
    nodes::NodeId,
    tabs_system::{EditorTab, EditorTabContext},
};
use bevy_egui::egui;

#[derive(Default)]
pub struct NodeInfoTab {
    pub node_id: Option<NodeId>,
}

impl NodeInfoTab {
    pub fn new(node_id: NodeId) -> Self {
        Self {
            node_id: Some(node_id),
        }
    }
}

impl EditorTab for NodeInfoTab {
    fn ui(&mut self, ui: &mut egui::Ui, context: &mut EditorTabContext) {
        let Some(node_id) = self.node_id else {
            ui.label("No node");
            return;
        };
        let g = &context.node_graph_res.0;
        let (n, geo) =
            cda::navigation::with_graph_by_path(&g, &context.node_editor_state.cda_path, |gg| {
                (
                    gg.nodes.get(&node_id).cloned(),
                    gg.geometry_cache.get(&node_id).cloned(),
                )
            });
        let Some(n) = n else {
            ui.label("Node not found");
            return;
        };
        let name = if n.name.is_empty() {
            n.node_type.name()
        } else {
            n.name.as_str()
        };

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(name).strong().size(18.0));
            ui.label(egui::RichText::new(n.node_type.name()).weak());
        });
        ui.separator();

        if let Some(geo) = geo.as_ref() {
            let pts = geo.get_point_count();
            let prims = geo.primitives().len();
            let verts = geo.vertices().len();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Points").weak());
                    ui.label(egui::RichText::new(format!("{pts}")).strong());
                });
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Primitives").weak());
                    ui.label(egui::RichText::new(format!("{prims}")).strong());
                });
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Vertices").weak());
                    ui.label(egui::RichText::new(format!("{verts}")).strong());
                });
            });

            if let Some((mn, mx)) = geo.compute_bounds() {
                let c = (mn + mx) * 0.5;
                let sz = mx - mn;
                ui.add_space(8.0);
                egui::Grid::new("bounds").num_columns(4).show(ui, |ui| {
                    ui.label("Center");
                    ui.label(format!("{:.3}", c.x));
                    ui.label(format!("{:.3}", c.y));
                    ui.label(format!("{:.3}", c.z));
                    ui.end_row();
                    ui.label("Size");
                    ui.label(format!("{:.3}", sz.x));
                    ui.label(format!("{:.3}", sz.y));
                    ui.label(format!("{:.3}", sz.z));
                    ui.end_row();
                    ui.label("Min");
                    ui.label(format!("{:.3}", mn.x));
                    ui.label(format!("{:.3}", mn.y));
                    ui.label(format!("{:.3}", mn.z));
                    ui.end_row();
                    ui.label("Max");
                    ui.label(format!("{:.3}", mx.x));
                    ui.label(format!("{:.3}", mx.y));
                    ui.label(format!("{:.3}", mx.z));
                    ui.end_row();
                });
            }

            ui.add_space(10.0);
            ui.collapsing("Point Attributes", |ui| {
                for (k, h) in &geo.point_attributes {
                    ui.label(format!("{}", k.as_str()));
                    ui.label(format!("{} ({})", k.as_str(), attr_kind(&h.data)));
                }
            });
        } else {
            ui.colored_label(
                egui::Color32::from_rgb(255, 200, 100),
                "This node has not cooked yet",
            );
            if ui.button("Cook Now").clicked() {
                let gg = &mut context.node_graph_res.0;
                cda::navigation::with_graph_by_path_mut(
                    gg,
                    &context.node_editor_state.cda_path,
                    |ggg| ggg.mark_dirty(node_id),
                );
                context.graph_changed_writer.write_default();
            }
        }
    }

    fn title(&self) -> egui::WidgetText {
        "Node Info".into()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn retained_key(&self, _ui: &egui::Ui, context: &EditorTabContext) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let id = self.node_id.map(|u| u.as_u128()).unwrap_or(0);
        context.graph_revision.hash(&mut hasher);
        (id as u64).hash(&mut hasher);
        ((id >> 64) as u64).hash(&mut hasher);
        context.node_editor_state.cda_path.len().hash(&mut hasher);
        for p in &context.node_editor_state.cda_path {
            p.hash(&mut hasher);
        }
        hasher.finish()
    }
}

fn attr_kind(a: &crate::mesh::Attribute) -> &'static str {
    use bevy::math::{DVec2, DVec3, DVec4};
    use bevy::prelude::{Vec2, Vec3, Vec4};
    if a.as_slice::<f32>().is_some() {
        "1-Flt"
    } else if a.as_slice::<Vec2>().is_some() {
        "2-Flt"
    } else if a.as_slice::<Vec3>().is_some() {
        "3-Flt"
    } else if a.as_slice::<Vec4>().is_some() {
        "4-Flt"
    } else if a.as_slice::<f64>().is_some() {
        "1-F64"
    } else if a.as_slice::<DVec2>().is_some() {
        "2-F64"
    } else if a.as_slice::<DVec3>().is_some() {
        "3-F64"
    } else if a.as_slice::<DVec4>().is_some() {
        "4-F64"
    } else if a.as_slice::<i32>().is_some() {
        "1-Int"
    } else if a.as_slice::<bool>().is_some() {
        "1-Bool"
    } else if a.as_slice::<String>().is_some() {
        "Str"
    } else {
        "?"
    }
}
