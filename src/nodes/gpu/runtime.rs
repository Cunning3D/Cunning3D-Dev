use crate::libs::geometry::attrs;
use crate::mesh::{Attribute, Geometry};
use bevy::prelude::Vec3;
use bytemuck::{Pod, Zeroable};
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParamsAffine {
    len: u32,
    _pad: [u32; 3],
    mul: [f32; 4],
    add: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct F4 {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

pub struct GpuRuntime {
    dev: wgpu::Device,
    q: wgpu::Queue,
    bgl_affine: wgpu::BindGroupLayout,
    ppl_affine: wgpu::ComputePipeline,
}

static GPU: OnceCell<GpuRuntime> = OnceCell::new();

fn build_affine_pipeline(dev: &wgpu::Device, bgl: &wgpu::BindGroupLayout) -> wgpu::ComputePipeline {
    let code = r#"
struct Params { len: u32, _pad: vec3<u32>, mul: vec4<f32>, add: vec4<f32> };
@group(0) @binding(0) var<storage, read_write> data: array<vec4<f32>>;
@group(0) @binding(1) var<uniform> params: Params;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let i = gid.x;
  if (i >= params.len) { return; }
  var v = data[i];
  v = vec4<f32>(v.xyz * params.mul.xyz + params.add.xyz, v.w);
  data[i] = v;
}
"#;
    let sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("c3d_gpu_affine_wgsl"),
        source: wgpu::ShaderSource::Wgsl(code.into()),
    });
    let pl = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("c3d_gpu_affine_pl"),
        bind_group_layouts: &[bgl],
        immediate_size: 0,
    });
    dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("c3d_gpu_affine_ppl"),
        layout: Some(&pl),
        module: &sm,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    })
}

impl GpuRuntime {
    #[cfg(target_arch = "wasm32")]
    pub fn get_blocking() -> &'static Self {
        GPU.get()
            .expect("GpuRuntime not initialized (call init_async_webgpu first)")
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn get_blocking() -> &'static Self {
        GPU.get_or_init(|| {
            let inst = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
            let adapter = pollster::block_on(inst.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            }))
            .expect("wgpu adapter");
            let (dev, q) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("c3d_gpu_runtime"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            }))
            .expect("wgpu device");
            let bgl_affine = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("c3d_gpu_affine_bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
            let ppl_affine = build_affine_pipeline(&dev, &bgl_affine);
            Self {
                dev,
                q,
                bgl_affine,
                ppl_affine,
            }
        })
    }

    #[inline]
    pub fn device(&self) -> &wgpu::Device {
        &self.dev
    }

    #[inline]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.q
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn init_async_webgpu() {
        if GPU.get().is_some() {
            return;
        }
        let inst = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = inst
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .expect("wgpu adapter");
        let (dev, q) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("c3d_gpu_runtime_web"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("wgpu device");
        let bgl_affine = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_gpu_affine_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let ppl_affine = build_affine_pipeline(&dev, &bgl_affine);
        let _ = GPU.set(Self {
            dev,
            q,
            bgl_affine,
            ppl_affine,
        });
    }
}

#[derive(Clone)]
pub struct GpuGeoHandle(pub(crate) Arc<GpuGeometry>);

pub(crate) struct GpuGeometry {
    pub(crate) cpu_base: Arc<Geometry>,
    pub(crate) point_count: usize,
    pub(crate) vec4_attrs: Mutex<HashMap<String, wgpu::Buffer>>, // vec4<f32> per point
    pub(crate) cpu_shadow: Mutex<Option<Arc<Geometry>>>,         // lazy readback cache
}

impl std::fmt::Debug for GpuGeoHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GpuGeoHandle")
    }
}

impl GpuGeoHandle {
    pub fn from_cpu(base: Arc<Geometry>) -> Self {
        let point_count = base.get_point_count();
        Self(Arc::new(GpuGeometry {
            cpu_base: base,
            point_count,
            vec4_attrs: Mutex::new(HashMap::new()),
            cpu_shadow: Mutex::new(None),
        }))
    }

    #[inline]
    pub fn point_count(&self) -> usize {
        self.0.point_count
    }
    #[inline]
    pub fn cpu_base(&self) -> Arc<Geometry> {
        self.0.cpu_base.clone()
    }

    fn ensure_vec4_attr_buffer(&self, attr: &str) -> wgpu::Buffer {
        if let Some(b) = self.0.vec4_attrs.lock().unwrap().get(attr).cloned() {
            return b;
        }
        let rt = GpuRuntime::get_blocking();
        let len = self.point_count();
        let src: Vec<Vec3> = self
            .cpu_base()
            .get_point_attribute(attr)
            .and_then(|a| a.as_slice::<Vec3>())
            .map(|s| s.to_vec())
            .unwrap_or_else(|| vec![Vec3::ZERO; len]);
        let buf: Vec<F4> = src
            .into_iter()
            .map(|p| F4 {
                x: p.x,
                y: p.y,
                z: p.z,
                w: 0.0,
            })
            .collect();
        let storage = rt
            .dev
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("c3d_gpu_attr_vec4"),
                contents: bytemuck::cast_slice(&buf),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            });
        self.0
            .vec4_attrs
            .lock()
            .unwrap()
            .insert(attr.to_string(), storage.clone());
        storage
    }

    fn list_vec4_attrs(&self) -> Vec<String> {
        self.0.vec4_attrs.lock().unwrap().keys().cloned().collect()
    }

    fn insert_vec4_attr_buffer(&self, attr: &str, buf: wgpu::Buffer) {
        self.0
            .vec4_attrs
            .lock()
            .unwrap()
            .insert(attr.to_string(), buf);
    }

    #[inline]
    pub fn is_points_only(&self) -> bool {
        let b = self.cpu_base();
        b.primitives().is_empty() && b.vertices().is_empty() && b.edges().is_empty()
    }

    pub fn apply_affine_vec3(&self, attr: &str, mul: Vec3, add: Vec3) -> Self {
        let rt = GpuRuntime::get_blocking();
        let storage = self.ensure_vec4_attr_buffer(attr);
        let params = ParamsAffine {
            len: self.point_count() as u32,
            _pad: [0; 3],
            mul: [mul.x, mul.y, mul.z, 0.0],
            add: [add.x, add.y, add.z, 0.0],
        };
        let ub = rt
            .dev
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("c3d_gpu_affine_params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bg = rt.dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3d_gpu_affine_bg"),
            layout: &rt.bgl_affine,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: storage.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: ub.as_entire_binding(),
                },
            ],
        });
        let mut enc = rt
            .dev
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("c3d_gpu_affine_enc"),
            });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("c3d_gpu_affine_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&rt.ppl_affine);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(((self.point_count() as u32) + 255) / 256, 1, 1);
        }
        rt.q.submit([enc.finish()]);
        *self.0.cpu_shadow.lock().unwrap() = None; // invalidate shadow
        self.clone()
    }

    pub fn download_blocking(&self) -> Arc<Geometry> {
        #[cfg(target_arch = "wasm32")]
        {
            // Non-blocking on wasm: schedule async readback (map_async callback) and return last shadow or CPU base.
            if let Some(g) = self.0.cpu_shadow.lock().unwrap().clone() {
                return g;
            }
            self.request_readback_async();
            return self.cpu_base();
        }
        if let Some(g) = self.0.cpu_shadow.lock().unwrap().clone() {
            return g;
        }
        let rt = GpuRuntime::get_blocking();
        let mut g = self.cpu_base().fork();
        let len = self.point_count();
        let bytes_len = (len * std::mem::size_of::<F4>()) as u64;
        let attrs: Vec<(String, wgpu::Buffer)> = self
            .0
            .vec4_attrs
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (attr, storage) in attrs {
            let out = rt.dev.create_buffer(&wgpu::BufferDescriptor {
                label: Some("c3d_gpu_readback"),
                size: bytes_len,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut enc = rt
                .dev
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("c3d_gpu_readback_enc"),
                });
            enc.copy_buffer_to_buffer(&storage, 0, &out, 0, bytes_len);
            rt.q.submit([enc.finish()]);
            let slice = out.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            let _ = rt.dev.poll(wgpu::PollType::wait_indefinitely());
            let mapped = slice.get_mapped_range();
            let outv: &[F4] = bytemuck::cast_slice(&mapped);
            let vv: Vec<Vec3> = outv.iter().map(|p| Vec3::new(p.x, p.y, p.z)).collect();
            drop(mapped);
            out.unmap();
            g.insert_point_attribute(attr.as_str(), Attribute::new(vv));
        }
        let g = Arc::new(g);
        *self.0.cpu_shadow.lock().unwrap() = Some(g.clone());
        g
    }

    #[inline]
    pub fn download_affine_attr_vec3_blocking(&self, attr: &str) -> Arc<Geometry> {
        // Ensure requested attribute exists on GPU before download (so CPU sees it).
        let _ = self.ensure_vec4_attr_buffer(attr);
        self.download_blocking()
    }

    #[cfg(target_arch = "wasm32")]
    fn request_readback_async(&self) {
        let Some(rt) = GPU.get() else {
            return;
        };
        let attrs: Vec<(String, wgpu::Buffer)> = self
            .0
            .vec4_attrs
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if attrs.is_empty() {
            return;
        }
        let len = self.point_count();
        let bytes_len = (len * std::mem::size_of::<F4>()) as u64;
        let cpu_base = self.cpu_base();
        let geo = self.0.clone();
        let results: Arc<Mutex<HashMap<String, Vec<Vec3>>>> = Arc::new(Mutex::new(HashMap::new()));
        let remaining = Arc::new(AtomicUsize::new(attrs.len()));

        for (attr, storage) in attrs {
            let out = rt.dev.create_buffer(&wgpu::BufferDescriptor {
                label: Some("c3d_gpu_readback_web"),
                size: bytes_len,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let out_c = out.clone();
            let mut enc = rt
                .dev
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("c3d_gpu_readback_enc_web"),
                });
            enc.copy_buffer_to_buffer(&storage, 0, &out, 0, bytes_len);
            rt.q.submit([enc.finish()]);
            let _ = rt.dev.poll(wgpu::PollType::Poll);

            let results = results.clone();
            let remaining = remaining.clone();
            let cpu_base = cpu_base.clone();
            let geo = geo.clone();
            let attr_k = attr.clone();
            out.slice(..).map_async(wgpu::MapMode::Read, move |res| {
                if res.is_ok() {
                    let slice = out_c.slice(..);
                    let mapped = slice.get_mapped_range();
                    let outv: &[F4] = bytemuck::cast_slice(&mapped);
                    let vv: Vec<Vec3> = outv.iter().map(|p| Vec3::new(p.x, p.y, p.z)).collect();
                    drop(mapped);
                    out_c.unmap();
                    results.lock().unwrap().insert(attr_k.clone(), vv);
                }
                if remaining.fetch_sub(1, Ordering::AcqRel) == 1 {
                    let mut g = cpu_base.fork();
                    for (k, v) in results.lock().unwrap().iter() {
                        g.insert_point_attribute(k.as_str(), Attribute::new(v.clone()));
                    }
                    *geo.cpu_shadow.lock().unwrap() = Some(Arc::new(g));
                }
            });
        }
    }
}

#[inline]
pub fn default_gpu_attr() -> &'static str {
    attrs::P
}

pub fn init_gpu_runtime_startup_system() {
    #[cfg(target_arch = "wasm32")]
    spawn_local(async {
        GpuRuntime::init_async_webgpu().await;
    });
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = GpuRuntime::get_blocking();
    }
}

pub fn merge_appendable_points(handles: &[GpuGeoHandle]) -> Option<GpuGeoHandle> {
    if handles.is_empty() {
        return None;
    }
    if !handles.iter().all(|h| h.is_points_only()) {
        return None;
    }
    let total: usize = handles.iter().map(|h| h.point_count()).sum();
    let mut base = Geometry::new();
    base.add_points_batch(total);
    let out = GpuGeoHandle::from_cpu(Arc::new(base));
    let rt = GpuRuntime::get_blocking();

    // Union of attrs (always include P).
    let mut attrs: std::collections::HashSet<String> = std::collections::HashSet::new();
    attrs.insert(default_gpu_attr().to_string());
    for h in handles {
        for a in h.list_vec4_attrs() {
            attrs.insert(a);
        }
    }

    let bytes_per = std::mem::size_of::<F4>() as u64;
    for attr in attrs {
        // Ensure all inputs have this buffer (upload from CPU if missing).
        let in_bufs: Vec<wgpu::Buffer> = handles
            .iter()
            .map(|h| h.ensure_vec4_attr_buffer(attr.as_str()))
            .collect();
        let out_bytes = (total as u64) * bytes_per;
        let out_buf = rt.dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_gpu_merge_vec4"),
            size: out_bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let mut enc = rt
            .dev
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("c3d_gpu_merge_enc"),
            });
        let mut dst_off: u64 = 0;
        for (h, b) in handles.iter().zip(in_bufs.iter()) {
            let sz = (h.point_count() as u64) * bytes_per;
            if sz != 0 {
                enc.copy_buffer_to_buffer(b, 0, &out_buf, dst_off, sz);
            }
            dst_off += sz;
        }
        rt.q.submit([enc.finish()]);
        out.insert_vec4_attr_buffer(attr.as_str(), out_buf);
    }
    Some(out)
}
