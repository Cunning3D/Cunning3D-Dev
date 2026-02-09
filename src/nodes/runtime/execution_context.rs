use crate::libs::geometry::geo_ref::ForEachMeta;
use crate::mesh::Geometry;
use crate::nodes::structs::GeoCacheRef;
use crate::nodes::{NodeId, PortId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

pub struct ExecutionContext {
    pub port_ref_cache: HashMap<(NodeId, PortId), GeoCacheRef>,
    pub foreach_ctx_stack: Vec<(String, ForEachMeta)>,
    pub externals_vec: Arc<Vec<GeoCacheRef>>,
    pub scratch_inputs: Vec<Arc<dyn crate::libs::geometry::geo_ref::GeometryRef>>,
    pub slot_cache: Vec<GeoCacheRef>,
    pub scratch_geos: Vec<Arc<Geometry>>,
    pub warnings: Vec<String>,
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self {
            port_ref_cache: HashMap::new(),
            foreach_ctx_stack: Vec::new(),
            externals_vec: Arc::new(Vec::new()),
            scratch_inputs: Vec::new(),
            slot_cache: Vec::new(),
            scratch_geos: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

pub struct CompiledExecutionContext {
    pub externals_vec: Arc<Vec<GeoCacheRef>>,
    pub slot_cache: Vec<GeoCacheRef>,
    pub scratch_inputs: Vec<Arc<dyn crate::libs::geometry::geo_ref::GeometryRef>>,
    pub scratch_geos: Vec<Arc<Geometry>>,
    pub scratch_gpu: Vec<crate::nodes::gpu::runtime::GpuGeoHandle>,
    pub warnings: Vec<String>,
    pub current_meta: ForEachMeta,
}

impl Default for CompiledExecutionContext {
    fn default() -> Self {
        Self {
            externals_vec: Arc::new(Vec::new()),
            slot_cache: Vec::new(),
            scratch_inputs: Vec::new(),
            scratch_geos: Vec::new(),
            scratch_gpu: Vec::new(),
            warnings: Vec::new(),
            current_meta: ForEachMeta::default(),
        }
    }
}

impl CompiledExecutionContext {
    #[inline]
    pub fn empty_geo() -> GeoCacheRef {
        GeoCacheRef::empty()
    }
    #[inline]
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }
}

thread_local! { static COMPILED_CTX_TLS: RefCell<CompiledExecutionContext> = RefCell::new(CompiledExecutionContext::default()); }

#[inline]
pub fn compiled_ctx_tls_init(externals_vec: Arc<Vec<GeoCacheRef>>, slot_count: usize) {
    COMPILED_CTX_TLS.with(|c| {
        let ctx = &mut *c.borrow_mut();
        ctx.externals_vec = externals_vec;
        if ctx.slot_cache.len() != slot_count {
            ctx.slot_cache.resize(slot_count, GeoCacheRef::empty());
        }
        ctx.scratch_inputs.clear();
        ctx.scratch_geos.clear();
        ctx.scratch_gpu.clear();
        ctx.warnings.clear();
        ctx.current_meta = ForEachMeta::default();
    });
}

#[inline]
pub fn with_compiled_ctx_tls<R>(f: impl FnOnce(&mut CompiledExecutionContext) -> R) -> R {
    COMPILED_CTX_TLS.with(|c| f(&mut *c.borrow_mut()))
}

impl ExecutionContext {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }
    #[inline]
    pub fn empty_geo() -> GeoCacheRef {
        GeoCacheRef::empty()
    }
    #[inline]
    pub fn push_foreach(&mut self, block_id: String, meta: ForEachMeta) {
        self.foreach_ctx_stack.push((block_id, meta));
    }
    #[inline]
    pub fn pop_foreach(&mut self) {
        self.foreach_ctx_stack.pop();
    }
    #[inline]
    pub fn warn(&mut self, msg: String) {
        self.warnings.push(msg);
    }
    #[inline]
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }
}
