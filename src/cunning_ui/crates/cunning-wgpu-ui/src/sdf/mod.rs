pub mod rect;
pub mod curve;
pub mod circle;
pub mod ellipse;
pub mod grid;
pub mod dashed_curve;
pub mod flow_curve;
pub mod quad;
pub mod tri;
pub mod text;
pub mod ui_stats;

#[derive(Clone, Copy, Debug, Default)]
pub struct SdfFrameId(pub u64);

#[derive(Clone, Copy, Debug, Default)]
pub struct SdfRenderSeq(pub u64);

#[derive(Clone, Copy, Debug, Default)]
pub struct SdfTime(pub f32);

pub(crate) fn clamp_scissor(info: &egui::PaintCallbackInfo, target_px: [u32; 2]) -> Option<(u32, u32, u32, u32)> {
    let tw = target_px[0];
    let th = target_px[1];
    if tw == 0 || th == 0 { return None; }
    let rect = info.viewport.intersect(info.clip_rect);
    let ppp = info.pixels_per_point.max(1e-6);
    // IMPORTANT: Use floor/ceil to avoid accidentally shaving off 1px on top/bottom during dragging or fractional DPI.
    let min_x = (rect.min.x * ppp).floor().max(0.0) as u32;
    let min_y = (rect.min.y * ppp).floor().max(0.0) as u32;
    let mut max_x = (rect.max.x * ppp).ceil().max(0.0) as u32;
    let mut max_y = (rect.max.y * ppp).ceil().max(0.0) as u32;
    if min_x >= tw || min_y >= th { return None; }
    if max_x > tw { max_x = tw; }
    if max_y > th { max_y = th; }
    if max_x <= min_x || max_y <= min_y { return None; }
    Some((min_x, min_y, max_x - min_x, max_y - min_y))
}

#[inline]
pub(crate) fn target_format(resources: &crate::CallbackResources) -> wgpu::TextureFormat {
    fn to_srgb(f: wgpu::TextureFormat) -> wgpu::TextureFormat {
        match f {
            wgpu::TextureFormat::Bgra8Unorm => wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8UnormSrgb,
            _ => f,
        }
    }
    to_srgb(resources.get::<crate::TargetFormat>().map(|v| v.0).unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb))
}

pub use rect::{SdfRectUniform, create_sdf_rect_callback};
pub use rect::{SdfRectBatchStats, SdfRectClipBatchStat, sdf_rect_last_batch_details, sdf_rect_last_stats, sdf_rect_set_verbose_details_enabled};
pub use curve::{SdfCurveUniform, create_sdf_curve_callback};
pub use circle::{SdfCircleUniform, create_sdf_circle_callback};
pub use ellipse::{SdfEllipseUniform, create_sdf_ellipse_callback};
pub use grid::{SdfGridUniform, create_sdf_grid_callback};
pub use dashed_curve::{SdfDashedCurveUniform, create_sdf_dashed_curve_callback};
pub use flow_curve::{SdfFlowCurveUniform, create_sdf_flow_curve_callback};
pub use quad::{SdfQuadUniform, create_sdf_quad_callback};
pub use tri::{SdfTriUniform, create_sdf_tri_callback};
pub use text::{GpuTextUniform, create_gpu_text_callback, GpuTextBatchStats, GpuTextClipBatchStat, gpu_text_last_batch_details, gpu_text_last_stats, gpu_text_set_verbose_details_enabled};
pub use text::{GpuTextTuning, gpu_text_tuning_get, gpu_text_tuning_set};
pub use ui_stats::{SdfUiBatchStats, sdf_ui_last_stats};

pub use grid::{SdfGridBatchStats, SdfGridClipBatchStat, sdf_grid_last_batch_details, sdf_grid_last_stats, sdf_grid_set_verbose_details_enabled};
pub use curve::{SdfCurveBatchStats, SdfCurveClipBatchStat, sdf_curve_last_batch_details, sdf_curve_last_stats, sdf_curve_set_verbose_details_enabled};
pub use dashed_curve::{SdfDashedCurveBatchStats, SdfDashedCurveClipBatchStat, sdf_dashed_curve_last_batch_details, sdf_dashed_curve_last_stats, sdf_dashed_curve_set_verbose_details_enabled};


