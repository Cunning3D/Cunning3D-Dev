//! CDA editor state management
use crate::cunning_core::cda::promoted_param::PromotedParam;
use crate::cunning_core::cda::CDAAsset;
use crate::nodes::structs::NodeId;
use bevy_egui::egui::Vec2;
use uuid::Uuid;

/// CDA edit stack level
#[derive(Clone, Debug)]
pub struct CDAEditLevel {
    pub cda_node_id: NodeId, // CDA node ID being edited
    pub parent_pan: Vec2,    // Pan before entering
    pub parent_zoom: f32,    // Zoom before entering
}

/// CDA property window tab
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CDAPropertyTab {
    #[default]
    Basic,
    Params,
    Exports,
    Overlay,
    Help,
    Icon,
}

/// Binding card tab
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BindingCardTab {
    #[default]
    Parameter,
    Channels,
    Menu,
}

/// CDA creation info
#[derive(Clone, Debug)]
pub struct CDACreateInfo {
    pub name: String,
    pub selected_nodes: Vec<NodeId>,
}

/// CDA editor state
#[derive(Clone, Default)]
pub struct CDAEditorState {
    // Edit stack (supports nested CDAs)
    pub edit_stack: Vec<CDAEditLevel>,

    // Create-CDA dialog
    pub create_dialog_open: bool,
    pub create_name: String,
    pub create_selected_nodes: Vec<NodeId>,
    pub pending_create: Option<CDACreateInfo>, // CDA pending creation

    // Property window
    pub property_window_open: bool,
    pub property_target: Option<NodeId>, // CDA node being edited
    pub property_tab: CDAPropertyTab,

    // Parameter editing
    pub selected_param_id: Option<Uuid>, // Selected promoted parameter ID
    pub binding_card_tab: BindingCardTab,
    pub params_folder_path: Vec<String>,
    pub param_clipboard: Option<PromotedParam>,

    // Drag state
    pub dragging_param: Option<DraggedParam>,
}

/// The parameter being dragged (deprecated; use egui DragAndDrop)
#[derive(Clone, Debug, Default)]
pub struct DraggedParam {
    pub source_node: NodeId,
    pub param_name: String,
    pub channel_index: Option<usize>,
    pub param_type: String, // Used for type-matching checks
}

impl CDAEditorState {
    /// Enter CDA edit mode
    pub fn enter_cda(&mut self, cda_node_id: NodeId, current_pan: Vec2, current_zoom: f32) {
        self.edit_stack.push(CDAEditLevel {
            cda_node_id,
            parent_pan: current_pan,
            parent_zoom: current_zoom,
        });
    }

    /// Exit current CDA edit and go back one level
    pub fn exit_cda(&mut self) -> Option<CDAEditLevel> {
        self.edit_stack.pop()
    }

    /// Get the CDA node ID currently being edited
    pub fn current_cda(&self) -> Option<NodeId> {
        self.edit_stack.last().map(|l| l.cda_node_id)
    }

    /// Get edit depth
    pub fn depth(&self) -> usize {
        self.edit_stack.len()
    }

    /// Whether we're editing inside a CDA
    pub fn is_editing_cda(&self) -> bool {
        !self.edit_stack.is_empty()
    }

    /// Get breadcrumb path
    pub fn breadcrumb(&self) -> Vec<NodeId> {
        self.edit_stack.iter().map(|l| l.cda_node_id).collect()
    }

    /// Open property window
    pub fn open_property_window(&mut self, cda_node_id: NodeId) {
        self.property_window_open = true;
        self.property_target = Some(cda_node_id);
        self.property_tab = CDAPropertyTab::Basic;
        self.selected_param_id = None;
    }

    /// Close property window
    pub fn close_property_window(&mut self) {
        self.property_window_open = false;
        self.property_target = None;
    }

    /// Open create dialog
    pub fn open_create_dialog(&mut self, selected_nodes: Vec<NodeId>) {
        self.create_dialog_open = true;
        self.create_name = "New CDA".to_string();
        self.create_selected_nodes = selected_nodes;
    }

    /// Close create dialog
    pub fn close_create_dialog(&mut self) {
        self.create_dialog_open = false;
        self.create_selected_nodes.clear();
    }

    /// Start dragging a parameter
    pub fn start_drag(&mut self, node: NodeId, param: &str, channel: Option<usize>, ptype: &str) {
        self.dragging_param = Some(DraggedParam {
            source_node: node,
            param_name: param.to_string(),
            channel_index: channel,
            param_type: ptype.to_string(),
        });
    }

    /// End drag
    pub fn end_drag(&mut self) -> Option<DraggedParam> {
        self.dragging_param.take()
    }

    /// Whether a drag is in progress
    pub fn is_dragging(&self) -> bool {
        self.dragging_param.is_some()
    }
}
