use bevy_egui::egui;

#[inline]
pub fn icon_for_node_name(_node_name: &str, _prefer_svg: bool) -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/options/gizmo.svg")
}
