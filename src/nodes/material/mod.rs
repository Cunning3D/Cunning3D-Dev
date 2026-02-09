pub mod material_assign;
pub mod pbr_material;
pub mod quick_material;

/// Default texture extensions for FilePath params.
#[inline]
pub(crate) fn tex_filters() -> Vec<String> {
    vec![
        "png", "jpg", "jpeg", "tga", "bmp", "hdr", "exr", "ktx2", "dds",
    ]
    .into_iter()
    .map(|s| s.into())
    .collect()
}
