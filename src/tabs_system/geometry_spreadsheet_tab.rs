use std::collections::{BTreeMap, BTreeSet};

use bevy::math::{DVec2, DVec3, DVec4};
use bevy::prelude::{Vec2, Vec3, Vec4};
use bevy_egui::egui::{Align, Color32, Layout, Ui, WidgetText};
use egui_extras::{Column, TableBuilder};

use crate::{
    mesh::{Attribute, Geometry},
    nodes::NodeId,
};

use super::{EditorTab, EditorTabContext};

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
enum SpreadsheetMode {
    #[default]
    Points,
    Vertices,
    Primitives,
    Edges,
    Detail,
}

#[derive(Default)]
pub struct GeometrySpreadsheetTab {
    mode: SpreadsheetMode,
    displayed_node_id: Option<NodeId>,
    displayed_geometry_id: u64,
    cached_display_data: Vec<Vec<String>>,
    cached_columns: Vec<DisplayColumn>,
    last_clicked_row: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct DisplayColumn {
    is_group: bool,
    base_name: String,
    component: Option<usize>,
}

impl DisplayColumn {
    fn display_name(&self) -> String {
        let name = self.base_name.replace('@', "");
        match self.component {
            Some(0) => format!("{}[x]", name),
            Some(1) => format!("{}[y]", name),
            Some(2) => format!("{}[z]", name),
            Some(3) => format!("{}[w]", name),
            _ => name,
        }
    }
}

impl EditorTab for GeometrySpreadsheetTab {
    fn ui(&mut self, ui: &mut Ui, context: &mut EditorTabContext) {
        ui.painter()
            .rect_filled(ui.clip_rect(), 0.0, ui.visuals().panel_fill);

        let target_node_id = context.ui_state.last_selected_node_id;
        let graph = &context.node_graph_res.0;

        let mut needs_update = false;

        if self.displayed_node_id != target_node_id {
            self.displayed_node_id = target_node_id;
            self.displayed_geometry_id = 0;
            needs_update = true;
        }

        if let Some(node_id) = self.displayed_node_id {
            if let Some(_) = graph.nodes.get(&node_id) {
                if let Some(geo) = graph.geometry_cache.get(&node_id) {
                    if geo.dirty_id != self.displayed_geometry_id {
                        needs_update = true;
                    }
                } else {
                    if self.displayed_geometry_id != 0 {
                        needs_update = true;
                    }
                }
            } else {
                self.displayed_node_id = None;
                if self.displayed_geometry_id != 0 {
                    needs_update = true;
                }
            }
        }

        if needs_update {
            if let Some(node_id) = self.displayed_node_id {
                if let Some(_) = graph.nodes.get(&node_id) {
                    if let Some(geo) = graph.geometry_cache.get(&node_id) {
                        self.displayed_geometry_id = geo.dirty_id;
                        self.rebuild_cache(geo);
                    } else {
                        self.displayed_geometry_id = 0;
                        self.cached_columns.clear();
                        self.cached_display_data.clear();
                    }
                }
            } else {
                self.displayed_geometry_id = 0;
                self.cached_columns.clear();
                self.cached_display_data.clear();
            }
        }

        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if ui
                    .selectable_value(&mut self.mode, SpreadsheetMode::Points, "Points")
                    .clicked()
                {
                    self.trigger_refresh(context, crate::ui::ComponentSelectionMode::Points);
                }
                if ui
                    .selectable_value(&mut self.mode, SpreadsheetMode::Vertices, "Vertices")
                    .clicked()
                {
                    self.trigger_refresh(context, crate::ui::ComponentSelectionMode::Vertices);
                }
                if ui
                    .selectable_value(&mut self.mode, SpreadsheetMode::Primitives, "Primitives")
                    .clicked()
                {
                    self.trigger_refresh(context, crate::ui::ComponentSelectionMode::Primitives);
                }
                if ui
                    .selectable_value(&mut self.mode, SpreadsheetMode::Edges, "Edges")
                    .clicked()
                {
                    self.trigger_refresh(context, crate::ui::ComponentSelectionMode::Edges);
                }
                if ui
                    .selectable_value(&mut self.mode, SpreadsheetMode::Detail, "Detail")
                    .clicked()
                {
                    self.displayed_geometry_id = 0;
                    context.ui_state.component_selection.indices.clear();
                }
            });
            ui.separator();

            if self.displayed_node_id.is_none() {
                ui.label("Select a node to view its geometry.");
            } else if self.cached_display_data.is_empty() && self.cached_columns.is_empty() {
                match self.mode {
                    SpreadsheetMode::Detail => ui.label("No detail attributes."),
                    SpreadsheetMode::Edges => {
                        ui.label("No explicit edges found (geometry may be implicit).")
                    }
                    _ => ui.label(format!("No {:?} in this geometry.", self.mode)),
                };
            } else {
                self.show_table_from_cache(ui, context);
            }
        });
    }

    fn title(&self) -> WidgetText {
        "Geometry Spreadsheet".into()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl GeometrySpreadsheetTab {
    fn trigger_refresh(
        &mut self,
        context: &mut EditorTabContext,
        mode: crate::ui::ComponentSelectionMode,
    ) {
        self.displayed_geometry_id = 0;
        context.ui_state.component_selection.indices.clear();
        context.ui_state.component_selection.mode = mode;
    }

    fn rebuild_cache(&mut self, geo: &Geometry) {
        self.cached_columns.clear();
        self.cached_display_data.clear();

        match self.mode {
            SpreadsheetMode::Points => {
                let point_count = geo.get_point_count();
                if point_count == 0 {
                    return;
                }
                let mut attr_columns = BTreeSet::new();
                for (name, value) in &geo.point_attributes {
                    add_display_columns_for_attr(&mut attr_columns, name.to_string(), &value.data);
                }

                for name in geo.point_groups.keys() {
                    attr_columns.insert(DisplayColumn {
                        is_group: true,
                        base_name: name.to_string(),
                        component: None,
                    });
                }

                let mut sorted_columns: Vec<DisplayColumn> = attr_columns.into_iter().collect();
                sorted_columns.sort_by(|a, b| {
                    if a.is_group != b.is_group {
                        a.is_group.cmp(&b.is_group)
                    } else if a.base_name == "@P" {
                        std::cmp::Ordering::Less
                    } else if b.base_name == "@P" {
                        std::cmp::Ordering::Greater
                    } else {
                        a.base_name
                            .cmp(&b.base_name)
                            .then(a.component.cmp(&b.component))
                    }
                });

                sorted_columns.insert(
                    0,
                    DisplayColumn {
                        is_group: false,
                        base_name: "Point".to_string(),
                        component: None,
                    },
                );
                self.cached_columns = sorted_columns;

                if !self.cached_columns.is_empty() {
                    for i in 0..point_count {
                        let mut row = vec![i.to_string()];
                        for col in self.cached_columns.iter().skip(1) {
                            if col.is_group {
                                let is_in_group = geo
                                    .get_point_group(&col.base_name)
                                    .map(|mask| mask.get(i))
                                    .unwrap_or(false);
                                row.push(if is_in_group {
                                    "1".to_string()
                                } else {
                                    "0".to_string()
                                });
                            } else {
                                let value_str = geo
                                    .get_point_attribute(col.base_name.as_str())
                                    .map(|attr| format_component(attr, col.component, i))
                                    .unwrap_or_default();
                                row.push(value_str);
                            }
                        }
                        self.cached_display_data.push(row);
                    }
                }
            }
            SpreadsheetMode::Vertices => {
                let mut attr_columns = BTreeSet::new();
                for (name, value) in &geo.vertex_attributes {
                    add_display_columns_for_attr(&mut attr_columns, name.to_string(), &value.data);
                }

                for name in geo.vertex_groups.keys() {
                    attr_columns.insert(DisplayColumn {
                        is_group: true,
                        base_name: name.to_string(),
                        component: None,
                    });
                }

                let mut sorted_columns: Vec<DisplayColumn> = attr_columns.into_iter().collect();
                sorted_columns.sort_by(|a, b| {
                    if a.is_group != b.is_group {
                        a.is_group.cmp(&b.is_group)
                    } else {
                        a.base_name
                            .cmp(&b.base_name)
                            .then(a.component.cmp(&b.component))
                    }
                });

                sorted_columns.insert(
                    0,
                    DisplayColumn {
                        is_group: false,
                        base_name: "Vertex".to_string(),
                        component: None,
                    },
                );
                sorted_columns.insert(
                    1,
                    DisplayColumn {
                        is_group: false,
                        base_name: "Point".to_string(),
                        component: None,
                    },
                );

                self.cached_columns = sorted_columns;

                if !self.cached_columns.is_empty() {
                    let num_vertices = geo.vertices().len();

                    for flat_vertex_idx in 0..num_vertices {
                        let vid = geo.vertices().get_id_from_dense(flat_vertex_idx).unwrap();
                        let vertex = geo.vertices().get(vid).unwrap();
                        let p_dense = geo
                            .points()
                            .get_dense_index(vertex.point_id.into())
                            .unwrap_or(0);

                        let mut row = vec![flat_vertex_idx.to_string(), p_dense.to_string()];
                        for col in self.cached_columns.iter().skip(2) {
                            if col.is_group {
                                let is_in_group = geo
                                    .get_vertex_group(&col.base_name)
                                    .map(|mask| mask.get(flat_vertex_idx))
                                    .unwrap_or(false);
                                row.push(if is_in_group {
                                    "1".to_string()
                                } else {
                                    "0".to_string()
                                });
                            } else {
                                let value_str = geo
                                    .get_vertex_attribute(col.base_name.as_str())
                                    .map(|attr| {
                                        format_component(attr, col.component, flat_vertex_idx)
                                    })
                                    .unwrap_or_default();
                                row.push(value_str);
                            }
                        }
                        self.cached_display_data.push(row);
                    }
                }
            }
            SpreadsheetMode::Primitives => {
                let primitive_count = geo.primitives().len();
                if primitive_count == 0 {
                    return;
                }

                let mut attr_columns = BTreeSet::new();
                for (name, value) in &geo.primitive_attributes {
                    add_display_columns_for_attr(&mut attr_columns, name.to_string(), &value.data);
                }

                for name in geo.primitive_groups.keys() {
                    attr_columns.insert(DisplayColumn {
                        is_group: true,
                        base_name: name.to_string(),
                        component: None,
                    });
                }

                let mut sorted_columns: Vec<DisplayColumn> = attr_columns.into_iter().collect();
                sorted_columns.sort_by(|a, b| {
                    if a.is_group != b.is_group {
                        a.is_group.cmp(&b.is_group)
                    } else {
                        a.base_name
                            .cmp(&b.base_name)
                            .then(a.component.cmp(&b.component))
                    }
                });

                sorted_columns.insert(
                    0,
                    DisplayColumn {
                        is_group: false,
                        base_name: "Primitive".to_string(),
                        component: None,
                    },
                );
                sorted_columns.insert(
                    1,
                    DisplayColumn {
                        is_group: false,
                        base_name: "Vertices".to_string(),
                        component: None,
                    },
                );

                self.cached_columns = sorted_columns;

                for i in 0..primitive_count {
                    let prim_id = geo.primitives().get_id_from_dense(i).unwrap();
                    let prim = geo.primitives().get(prim_id).unwrap();

                    let mut row = vec![i.to_string(), prim.vertices().len().to_string()];
                    for col in self.cached_columns.iter().skip(2) {
                        if col.is_group {
                            let is_in_group = geo
                                .get_primitive_group(&col.base_name)
                                .map(|mask| mask.get(i))
                                .unwrap_or(false);
                            row.push(if is_in_group {
                                "1".to_string()
                            } else {
                                "0".to_string()
                            });
                        } else {
                            let value_str = geo
                                .get_primitive_attribute(col.base_name.as_str())
                                .map(|attr| format_component(attr, col.component, i))
                                .unwrap_or_default();
                            row.push(value_str);
                        }
                    }
                    self.cached_display_data.push(row);
                }
            }
            SpreadsheetMode::Edges => {
                let edge_count = geo.edges().len();
                if edge_count == 0 {
                    return;
                }

                let mut attr_columns = BTreeSet::new();
                for (name, value) in &geo.edge_attributes {
                    add_display_columns_for_attr(&mut attr_columns, name.to_string(), &value.data);
                }

                if let Some(groups) = &geo.edge_groups {
                    for name in groups.keys() {
                        attr_columns.insert(DisplayColumn {
                            is_group: true,
                            base_name: name.to_string(),
                            component: None,
                        });
                    }
                }

                let mut sorted_columns: Vec<DisplayColumn> = attr_columns.into_iter().collect();
                sorted_columns.sort_by(|a, b| {
                    if a.is_group != b.is_group {
                        a.is_group.cmp(&b.is_group)
                    } else {
                        a.base_name
                            .cmp(&b.base_name)
                            .then(a.component.cmp(&b.component))
                    }
                });

                sorted_columns.insert(
                    0,
                    DisplayColumn {
                        is_group: false,
                        base_name: "Edge".to_string(),
                        component: None,
                    },
                );
                sorted_columns.insert(
                    1,
                    DisplayColumn {
                        is_group: false,
                        base_name: "P0".to_string(),
                        component: None,
                    },
                );
                sorted_columns.insert(
                    2,
                    DisplayColumn {
                        is_group: false,
                        base_name: "P1".to_string(),
                        component: None,
                    },
                );

                self.cached_columns = sorted_columns;

                for i in 0..edge_count {
                    let edge_id = geo.edges().get_id_from_dense(i).unwrap();
                    let edge = geo.edges().get(edge_id).unwrap();
                    let p0_dense = geo.points().get_dense_index(edge.p0.into()).unwrap_or(0);
                    let p1_dense = geo.points().get_dense_index(edge.p1.into()).unwrap_or(0);

                    let mut row = vec![i.to_string(), p0_dense.to_string(), p1_dense.to_string()];
                    for col in self.cached_columns.iter().skip(3) {
                        if col.is_group {
                            let is_in_group = geo
                                .get_edge_group(&col.base_name)
                                .map(|mask| mask.get(i))
                                .unwrap_or(false);
                            row.push(if is_in_group {
                                "1".to_string()
                            } else {
                                "0".to_string()
                            });
                        } else {
                            let value_str = geo
                                .get_edge_attribute(col.base_name.as_str())
                                .map(|attr| format_component(attr, col.component, i))
                                .unwrap_or_default();
                            row.push(value_str);
                        }
                    }
                    self.cached_display_data.push(row);
                }
            }
            SpreadsheetMode::Detail => {
                self.cached_columns = vec![
                    DisplayColumn {
                        is_group: false,
                        base_name: "Attribute".to_string(),
                        component: None,
                    },
                    DisplayColumn {
                        is_group: false,
                        base_name: "Value".to_string(),
                        component: None,
                    },
                ];
                let sorted_attrs: BTreeMap<_, _> = geo.detail_attributes.iter().collect();
                self.cached_display_data = sorted_attrs
                    .into_iter()
                    .map(|(name, value)| vec![name.to_string(), format_attribute(&value.data)])
                    .collect();
            }
        }
    }

    fn show_table_from_cache(&mut self, ui: &mut Ui, context: &mut EditorTabContext) {
        let mut table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(Layout::left_to_right(Align::Center));

        for (i, col) in self.cached_columns.iter().enumerate() {
            let initial_width = if i == 0 { 60.0 } else { 80.0 };
            let min_width = if i == 0 { 40.0 } else { 60.0 };
            if col.base_name == "Value" {
                table = table.column(Column::remainder().at_least(100.0));
            } else {
                table = table.column(Column::initial(initial_width).at_least(min_width));
            }
        }

        let num_rows = self.cached_display_data.len();
        let row_height = 18.0;

        table
            .header(20.0, |mut header| {
                for col in &self.cached_columns {
                    header.col(|ui| {
                        if col.is_group {
                            ui.colored_label(Color32::from_rgb(100, 200, 255), col.display_name());
                        } else {
                            ui.strong(col.display_name());
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(row_height, num_rows, |mut row| {
                    let row_index = row.index();
                    if let Some(row_data) = self.cached_display_data.get(row_index) {
                        row.col(|ui| {
                            let is_selected = context
                                .ui_state
                                .component_selection
                                .indices
                                .contains(&row_index);
                            let response = ui.selectable_label(is_selected, &row_data[0]);
                            if response.clicked() {
                                handle_row_selection(
                                    ui,
                                    row_index,
                                    is_selected,
                                    &mut self.last_clicked_row,
                                    context.ui_state,
                                );
                            }
                        });

                        for (_col_index, cell_text) in row_data.iter().enumerate().skip(1) {
                            row.col(|ui| {
                                ui.label(cell_text);
                            });
                        }
                    }
                });
            });
    }
}

fn handle_row_selection(
    ui: &Ui,
    row_index: usize,
    is_selected: bool,
    last_clicked_row: &mut Option<usize>,
    ui_state: &mut crate::ui::UiState,
) {
    let modifiers = ui.input(|i| i.modifiers);
    if modifiers.shift {
        if let Some(last_clicked) = *last_clicked_row {
            let range_start = last_clicked.min(row_index);
            let range_end = last_clicked.max(row_index);
            for i in range_start..=range_end {
                ui_state.component_selection.indices.insert(i);
            }
        } else {
            ui_state.component_selection.indices.clear();
            ui_state.component_selection.indices.insert(row_index);
        }
    } else if modifiers.ctrl {
        if is_selected {
            ui_state.component_selection.indices.remove(&row_index);
        } else {
            ui_state.component_selection.indices.insert(row_index);
        }
    } else {
        ui_state.component_selection.indices.clear();
        ui_state.component_selection.indices.insert(row_index);
    }
    *last_clicked_row = Some(row_index);
}

fn add_display_columns_for_attr(
    columns: &mut BTreeSet<DisplayColumn>,
    name: String,
    value: &Attribute,
) {
    if value.as_slice::<Vec3>().is_some() || value.as_slice::<DVec3>().is_some() {
        columns.insert(DisplayColumn {
            is_group: false,
            base_name: name.clone(),
            component: Some(0),
        });
        columns.insert(DisplayColumn {
            is_group: false,
            base_name: name.clone(),
            component: Some(1),
        });
        columns.insert(DisplayColumn {
            is_group: false,
            base_name: name,
            component: Some(2),
        });
    } else if value.as_slice::<Vec4>().is_some() || value.as_slice::<DVec4>().is_some() {
        columns.insert(DisplayColumn {
            is_group: false,
            base_name: name.clone(),
            component: Some(0),
        });
        columns.insert(DisplayColumn {
            is_group: false,
            base_name: name.clone(),
            component: Some(1),
        });
        columns.insert(DisplayColumn {
            is_group: false,
            base_name: name.clone(),
            component: Some(2),
        });
        columns.insert(DisplayColumn {
            is_group: false,
            base_name: name,
            component: Some(3),
        });
    } else if value.as_slice::<Vec2>().is_some() || value.as_slice::<DVec2>().is_some() {
        columns.insert(DisplayColumn {
            is_group: false,
            base_name: name.clone(),
            component: Some(0),
        });
        columns.insert(DisplayColumn {
            is_group: false,
            base_name: name,
            component: Some(1),
        });
    } else {
        columns.insert(DisplayColumn {
            is_group: false,
            base_name: name,
            component: None,
        });
    }
}

fn format_component(attr: &Attribute, component: Option<usize>, index: usize) -> String {
    if let Some(v) = attr.as_slice::<f32>() {
        return v.get(index).map_or("".to_string(), |f| format!("{:.3}", f));
    }
    if let Some(v) = attr.as_slice::<f64>() {
        return v.get(index).map_or("".to_string(), |f| format!("{:.3}", f));
    }
    if let Some(v) = attr.as_slice::<i32>() {
        return v.get(index).map_or("".to_string(), |f| format!("{}", f));
    }
    if let Some(v) = attr.as_slice::<bool>() {
        return v.get(index).map_or("".to_string(), |f| format!("{}", f));
    }
    if let Some(v) = attr.as_slice::<String>() {
        return v.get(index).cloned().unwrap_or_default();
    }

    if let Some(v) = attr.as_slice::<Vec3>() {
        return match component {
            Some(0) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.x)),
            Some(1) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.y)),
            Some(2) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.z)),
            _ => "".to_string(),
        };
    }
    if let Some(v) = attr.as_slice::<DVec3>() {
        return match component {
            Some(0) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.x)),
            Some(1) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.y)),
            Some(2) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.z)),
            _ => "".to_string(),
        };
    }
    if let Some(v) = attr.as_slice::<Vec2>() {
        return match component {
            Some(0) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.x)),
            Some(1) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.y)),
            _ => "".to_string(),
        };
    }
    if let Some(v) = attr.as_slice::<DVec2>() {
        return match component {
            Some(0) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.x)),
            Some(1) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.y)),
            _ => "".to_string(),
        };
    }
    if let Some(v) = attr.as_slice::<Vec4>() {
        return match component {
            Some(0) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.x)),
            Some(1) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.y)),
            Some(2) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.z)),
            Some(3) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.w)),
            _ => "".to_string(),
        };
    }
    if let Some(v) = attr.as_slice::<DVec4>() {
        return match component {
            Some(0) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.x)),
            Some(1) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.y)),
            Some(2) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.z)),
            Some(3) => v
                .get(index)
                .map_or("".to_string(), |vec| format!("{:.3}", vec.w)),
            _ => "".to_string(),
        };
    }

    "".to_string()
}

fn format_attribute(attr: &Attribute) -> String {
    if let Some(v) = attr.as_slice::<f32>() {
        return format!("F32[{}]", v.len());
    }
    if let Some(v) = attr.as_slice::<f64>() {
        return format!("F64[{}]", v.len());
    }
    if let Some(v) = attr.as_slice::<Vec2>() {
        return format!("Vec2[{}]", v.len());
    }
    if let Some(v) = attr.as_slice::<DVec2>() {
        return format!("DVec2[{}]", v.len());
    }
    if let Some(v) = attr.as_slice::<Vec3>() {
        return format!("Vec3[{}]", v.len());
    }
    if let Some(v) = attr.as_slice::<DVec3>() {
        return format!("DVec3[{}]", v.len());
    }
    if let Some(v) = attr.as_slice::<Vec4>() {
        return format!("Vec4[{}]", v.len());
    }
    if let Some(v) = attr.as_slice::<DVec4>() {
        return format!("DVec4[{}]", v.len());
    }
    if let Some(v) = attr.as_slice::<i32>() {
        return format!("I32[{}]", v.len());
    }
    if let Some(v) = attr.as_slice::<bool>() {
        return format!("Bool[{}]", v.len());
    }
    if let Some(v) = attr.as_slice::<String>() {
        return format!("String[{}]", v.len());
    }
    "Unknown".to_string()
}
