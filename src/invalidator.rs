use bevy::prelude::*;
use std::sync::OnceLock;
use std::time::Duration;

fn dbg_repaint() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("DCC_LOG_REPAINT").ok().as_deref() == Some("1"))
}

fn dbg_graph_changed() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("DCC_LOG_GRAPH_CHANGED").ok().as_deref() == Some("1"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepaintCause {
    /// Input event (mouse move, click, key press)
    Input,
    /// Animation in progress (hover fade, node translation)
    Animation,
    /// Data/Model changed (node graph, selection, property)
    DataChanged,
    /// Layout changed (resize, dock adjustment)
    Layout,
    /// Debugging/Profiling
    Debug,
}

/// Monotonic revision counter for graph-level UI invalidation.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct GraphRevision(pub u64);

/// Resource to manage UI repaint requests in a centralized way.
/// Replaces ad-hoc `ctx.request_repaint()` calls.
#[derive(Resource, Default)]
pub struct UiInvalidator {
    needs_repaint: bool,
    causes: Vec<RepaintCause>,
    delay: Option<Duration>,
    last_causes: Vec<RepaintCause>,
    last_delay: Option<Duration>,
    requests: u64,
    last_tag: &'static str,
    last_log_hash: u64,
    last_log_time_s: f64,
}

impl UiInvalidator {
    #[inline]
    fn request_common(&mut self, delay: Option<Duration>, cause: RepaintCause) {
        self.needs_repaint = true;
        self.requests = self.requests.wrapping_add(1);
        if let Some(delay) = delay {
            self.delay = Some(self.delay.map_or(delay, |d| d.min(delay)));
        }
        if !self.causes.contains(&cause) {
            self.causes.push(cause);
        }
    }

    pub fn request_repaint(&mut self, cause: RepaintCause) {
        self.last_tag = "unknown";
        self.request_common(None, cause);
    }

    pub fn request_repaint_after(&mut self, delay: Duration, cause: RepaintCause) {
        self.last_tag = "unknown";
        self.request_common(Some(delay), cause);
    }

    pub fn request_repaint_tagged(&mut self, tag: &'static str, cause: RepaintCause) {
        self.last_tag = tag;
        self.request_common(None, cause);
        if dbg_repaint() {
            bevy::log::warn!("[Repaint] tag={} delay=0 cause={:?}", tag, cause);
        }
    }

    pub fn request_repaint_after_tagged(
        &mut self,
        tag: &'static str,
        delay: Duration,
        cause: RepaintCause,
    ) {
        self.last_tag = tag;
        self.request_common(Some(delay), cause);
        if dbg_repaint() {
            bevy::log::warn!(
                "[Repaint] tag={} delay_ms={} cause={:?}",
                tag,
                delay.as_millis(),
                cause
            );
        }
    }

    pub fn check_and_clear(&mut self) -> Option<Vec<RepaintCause>> {
        if self.needs_repaint {
            self.needs_repaint = false;
            self.last_delay = self.delay;
            self.delay = None;
            let causes = std::mem::take(&mut self.causes);
            self.last_causes = causes.clone();
            Some(causes)
        } else {
            None
        }
    }

    pub fn debug_state(&self) -> (u64, Option<Duration>, &[RepaintCause]) {
        (self.requests, self.last_delay, &self.last_causes)
    }

    pub fn debug_tag(&self) -> &'static str {
        self.last_tag
    }
}

/// System to apply the invalidator state to egui contexts.
pub fn apply_invalidator_system(
    mut invalidator: ResMut<UiInvalidator>,
    mut egui_contexts: Query<&mut bevy_egui::EguiContext>,
    time: Res<Time>,
) {
    if !invalidator.needs_repaint {
        return;
    }
    let delay = invalidator.delay;
    let _causes = invalidator.check_and_clear();
    let _ = time; // reserved for future profiling hooks
    for mut ctx in egui_contexts.iter_mut() {
        let c = ctx.get_mut();
        match delay {
            Some(d) if d != Duration::ZERO => c.request_repaint_after(d),
            _ => c.request_repaint(),
        }
    }
}

/// Bump graph revision whenever `GraphChanged` fires, and request a repaint.
/// Also handles the new `UiChanged` event for UI-only repaints.
pub fn bump_graph_revision_system(
    mut ev_graph: MessageReader<crate::GraphChanged>,
    mut ev_ui: MessageReader<crate::UiChanged>,
    mut ev_geo: MessageReader<crate::GeometryChanged>,
    mut rev: ResMut<GraphRevision>,
    mut inv: ResMut<UiInvalidator>,
) {
    puffin::profile_function!();
    
    // Count legacy GraphChanged events
    let mut n_graph = 0u64;
    for _ in ev_graph.read() {
        n_graph += 1;
    }
    
    // Count new UiChanged events
    let mut n_ui = 0u64;
    for _ in ev_ui.read() {
        n_ui += 1;
    }
    
    // Count new GeometryChanged events
    let mut n_geo = 0u64;
    for _ in ev_geo.read() {
        n_geo += 1;
    }
    
    let total = n_graph + n_ui + n_geo;
    if total != 0 {
        puffin::profile_scope!("invalidator_events", format!("graph={} ui={} geo={}", n_graph, n_ui, n_geo));
        rev.0 = rev.0.saturating_add(total);
        inv.request_repaint_tagged("graph/changed", RepaintCause::DataChanged);
        if dbg_graph_changed() {
            bevy::log::info!(
                "[Invalidator] graph={} ui={} geo={} total_rev={}",
                n_graph, n_ui, n_geo, rev.0
            );
        }
    }
}

/// Plugin to setup invalidator
pub struct InvalidatorPlugin;

impl Plugin for InvalidatorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiInvalidator>()
            .init_resource::<GraphRevision>()
            // IMPORTANT: Must run before bevy_egui's ProcessOutput (end_frame) so repaint requests are observed in the same frame.
            .add_systems(
                PostUpdate,
                (bump_graph_revision_system, apply_invalidator_system)
                    .chain()
                    .before(bevy_egui::EguiSet::ProcessOutput),
            );
    }
}
