use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Default)]
pub struct SdfUiBatchStats {
    pub frame_id: u64,
    pub rects: u64,
    pub circles: u64,
    pub ellipses: u64,
    pub curves: u64,
    pub clip_runs: u64,
    pub drawcalls: u64,
}

static SDF_UI_LAST_FRAME: AtomicU64 = AtomicU64::new(0);
static SDF_UI_LAST_RECTS: AtomicU64 = AtomicU64::new(0);
static SDF_UI_LAST_CIRCLES: AtomicU64 = AtomicU64::new(0);
static SDF_UI_LAST_ELLIPSES: AtomicU64 = AtomicU64::new(0);
static SDF_UI_LAST_CURVES: AtomicU64 = AtomicU64::new(0);
static SDF_UI_LAST_CLIP_RUNS: AtomicU64 = AtomicU64::new(0);
static SDF_UI_LAST_DRAWCALLS: AtomicU64 = AtomicU64::new(0);

pub fn sdf_ui_last_stats() -> SdfUiBatchStats {
    SdfUiBatchStats {
        frame_id: SDF_UI_LAST_FRAME.load(Ordering::Relaxed),
        rects: SDF_UI_LAST_RECTS.load(Ordering::Relaxed),
        circles: SDF_UI_LAST_CIRCLES.load(Ordering::Relaxed),
        ellipses: SDF_UI_LAST_ELLIPSES.load(Ordering::Relaxed),
        curves: SDF_UI_LAST_CURVES.load(Ordering::Relaxed),
        clip_runs: SDF_UI_LAST_CLIP_RUNS.load(Ordering::Relaxed),
        drawcalls: SDF_UI_LAST_DRAWCALLS.load(Ordering::Relaxed),
    }
}

pub fn sdf_ui_set_stats(st: SdfUiBatchStats) {
    SDF_UI_LAST_FRAME.store(st.frame_id, Ordering::Relaxed);
    SDF_UI_LAST_RECTS.store(st.rects, Ordering::Relaxed);
    SDF_UI_LAST_CIRCLES.store(st.circles, Ordering::Relaxed);
    SDF_UI_LAST_ELLIPSES.store(st.ellipses, Ordering::Relaxed);
    SDF_UI_LAST_CURVES.store(st.curves, Ordering::Relaxed);
    SDF_UI_LAST_CLIP_RUNS.store(st.clip_runs, Ordering::Relaxed);
    SDF_UI_LAST_DRAWCALLS.store(st.drawcalls, Ordering::Relaxed);
}







