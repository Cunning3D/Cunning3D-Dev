use bevy_egui::egui;
use std::borrow::Cow;

const P: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
    0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
    0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
    0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
    0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

#[inline]
const fn icon(uri: &'static str) -> egui::ImageSource<'static> {
    egui::ImageSource::Bytes { uri: Cow::Borrowed(uri), bytes: egui::load::Bytes::Static(P) }
}

pub struct ViewportIcons;

impl ViewportIcons {
    pub const SHADED_WIRE: egui::ImageSource<'static> = icon("bytes://viewport/shaded_wire");
    pub const WIREFRAME: egui::ImageSource<'static> = icon("bytes://viewport/wireframe");
    pub const SHADED: egui::ImageSource<'static> = icon("bytes://viewport/shaded");
    pub const EXPAND: egui::ImageSource<'static> = icon("bytes://viewport/expand");
    pub const COLLAPSE: egui::ImageSource<'static> = icon("bytes://viewport/collapse");
    pub const GIZMO: egui::ImageSource<'static> = icon("bytes://viewport/gizmo");
    pub const GRID: egui::ImageSource<'static> = icon("bytes://viewport/grid");
    pub const POINTS: egui::ImageSource<'static> = icon("bytes://viewport/points");
    pub const POINT_NUMS: egui::ImageSource<'static> = icon("bytes://viewport/point_nums");
    pub const VERT_NUMS: egui::ImageSource<'static> = icon("bytes://viewport/vert_nums");
    pub const VERT_NORMS: egui::ImageSource<'static> = icon("bytes://viewport/vert_norms");
    pub const PRIM_NUMS: egui::ImageSource<'static> = icon("bytes://viewport/prim_nums");
    pub const PRIM_NORMS: egui::ImageSource<'static> = icon("bytes://viewport/prim_norms");
}

