//! CDA编辑状态管理
use crate::cunning_core::cda::promoted_param::PromotedParam;
use crate::cunning_core::cda::CDAAsset;
use crate::nodes::structs::NodeId;
use bevy_egui::egui::Vec2;
use uuid::Uuid;

/// CDA编辑栈层级
#[derive(Clone, Debug)]
pub struct CDAEditLevel {
    pub cda_node_id: NodeId, // 正在编辑的CDA节点ID
    pub parent_pan: Vec2,    // 进入前的pan
    pub parent_zoom: f32,    // 进入前的zoom
}

/// CDA属性窗口标签页
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

/// 绑定卡片标签页
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BindingCardTab {
    #[default]
    Parameter,
    Channels,
    Menu,
}

/// CDA创建信息
#[derive(Clone, Debug)]
pub struct CDACreateInfo {
    pub name: String,
    pub selected_nodes: Vec<NodeId>,
}

/// CDA编辑器状态
#[derive(Clone, Default)]
pub struct CDAEditorState {
    // 编辑栈（支持嵌套CDA）
    pub edit_stack: Vec<CDAEditLevel>,

    // 创建CDA对话框
    pub create_dialog_open: bool,
    pub create_name: String,
    pub create_selected_nodes: Vec<NodeId>,
    pub pending_create: Option<CDACreateInfo>, // 待创建的CDA

    // 属性窗口
    pub property_window_open: bool,
    pub property_target: Option<NodeId>, // 正在编辑的CDA节点
    pub property_tab: CDAPropertyTab,

    // 参数编辑
    pub selected_param_id: Option<Uuid>, // 选中的提升参数ID
    pub binding_card_tab: BindingCardTab,
    pub params_folder_path: Vec<String>,
    pub param_clipboard: Option<PromotedParam>,

    // 拖拽状态
    pub dragging_param: Option<DraggedParam>,
}

/// 正在拖拽的参数（废弃，改用egui DragAndDrop）
#[derive(Clone, Debug, Default)]
pub struct DraggedParam {
    pub source_node: NodeId,
    pub param_name: String,
    pub channel_index: Option<usize>,
    pub param_type: String, // 用于类型匹配检查
}

impl CDAEditorState {
    /// 进入CDA编辑
    pub fn enter_cda(&mut self, cda_node_id: NodeId, current_pan: Vec2, current_zoom: f32) {
        self.edit_stack.push(CDAEditLevel {
            cda_node_id,
            parent_pan: current_pan,
            parent_zoom: current_zoom,
        });
    }

    /// 退出当前CDA编辑，返回上一层
    pub fn exit_cda(&mut self) -> Option<CDAEditLevel> {
        self.edit_stack.pop()
    }

    /// 获取当前编辑的CDA节点ID
    pub fn current_cda(&self) -> Option<NodeId> {
        self.edit_stack.last().map(|l| l.cda_node_id)
    }

    /// 获取编辑深度
    pub fn depth(&self) -> usize {
        self.edit_stack.len()
    }

    /// 是否在CDA内部编辑模式
    pub fn is_editing_cda(&self) -> bool {
        !self.edit_stack.is_empty()
    }

    /// 获取面包屑路径
    pub fn breadcrumb(&self) -> Vec<NodeId> {
        self.edit_stack.iter().map(|l| l.cda_node_id).collect()
    }

    /// 打开属性窗口
    pub fn open_property_window(&mut self, cda_node_id: NodeId) {
        self.property_window_open = true;
        self.property_target = Some(cda_node_id);
        self.property_tab = CDAPropertyTab::Basic;
        self.selected_param_id = None;
    }

    /// 关闭属性窗口
    pub fn close_property_window(&mut self) {
        self.property_window_open = false;
        self.property_target = None;
    }

    /// 打开创建对话框
    pub fn open_create_dialog(&mut self, selected_nodes: Vec<NodeId>) {
        self.create_dialog_open = true;
        self.create_name = "New CDA".to_string();
        self.create_selected_nodes = selected_nodes;
    }

    /// 关闭创建对话框
    pub fn close_create_dialog(&mut self) {
        self.create_dialog_open = false;
        self.create_selected_nodes.clear();
    }

    /// 开始拖拽参数
    pub fn start_drag(&mut self, node: NodeId, param: &str, channel: Option<usize>, ptype: &str) {
        self.dragging_param = Some(DraggedParam {
            source_node: node,
            param_name: param.to_string(),
            channel_index: channel,
            param_type: ptype.to_string(),
        });
    }

    /// 结束拖拽
    pub fn end_drag(&mut self) -> Option<DraggedParam> {
        self.dragging_param.take()
    }

    /// 是否正在拖拽
    pub fn is_dragging(&self) -> bool {
        self.dragging_param.is_some()
    }
}
