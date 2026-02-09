//! GPU voxel meshing (prototype).
//!
//! Goal: Voxy-style GPU pipeline building draw-ready face instances.
//! This first step generates *unmerged* faces (one quad per exposed voxel face),
//! which is enough to validate correctness and performance of the GPU path.
//!
//! Next steps (not in this file yet):
//! - greedy-merge on GPU to reduce face count
//! - keep persistent GPU voxel storage per chunk (avoid per-cook uploads)
//! - render directly via indirect draw (avoid CPU readback)

use bevy::prelude::*;
use bytemuck::{Pod, Zeroable};
use once_cell::sync::OnceCell;
use std::sync::Mutex;

use crate::nodes::gpu::runtime::GpuRuntime;
use crate::mesh::{Attribute, GeoPrimitive, Geometry, PointId, PolygonPrim};
use crate::libs::geometry::attrs;
use cunning_kernel::algorithms::algorithms_editor::voxel as vox;

/// Padded chunk dimension: CHUNK_SIZE + 2 (one-voxel border).
pub const PAD: i32 = 1;
pub const CHUNK_SIZE: i32 = crate::volume::CHUNK_SIZE;
pub const PADDED: i32 = CHUNK_SIZE + 2;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Params {
    padded_dim: u32,
    chunk_dim: u32,
    max_out: u32,
    _pad0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct OutHeader {
    count: u32,
    _pad0: [u32; 3],
    _pad1: [u32; 4],
}

/// One exposed face (quad) instance.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug)]
pub struct FaceInstance {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// 0..5: +X,-X,+Y,-Y,+Z,-Z
    pub dir: u32,
    /// palette index (0 excluded)
    pub pi: u32,
    _pad: [u32; 3],
}

pub struct GpuVoxelMesher {
    bgl: wgpu::BindGroupLayout,
    ppl: wgpu::ComputePipeline,
    scratch: Mutex<GpuVoxelMesherScratch>,
}

#[derive(Default)]
struct GpuVoxelMesherScratch {
    param: Option<wgpu::Buffer>,
    vox: Option<wgpu::Buffer>,
    header: Option<wgpu::Buffer>,
    faces: Option<wgpu::Buffer>,
    read_header: Option<wgpu::Buffer>,
    read_faces: Option<wgpu::Buffer>,
    bg: Option<wgpu::BindGroup>,
    vox_len_u32: usize,
    max_out: u32,
}

#[inline]
fn ensure_buf(dev: &wgpu::Device, cur: &mut Option<wgpu::Buffer>, label: &'static str, size: u64, usage: wgpu::BufferUsages) {
    let need = cur.as_ref().map(|b| b.size() < size).unwrap_or(true);
    if need {
        *cur = Some(dev.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false }));
    }
}

impl GpuVoxelMesher {
    pub fn global(rt: &GpuRuntime) -> &'static Self {
        static M: OnceCell<GpuVoxelMesher> = OnceCell::new();
        M.get_or_init(|| Self::new(rt))
    }

    pub fn new(rt: &GpuRuntime) -> Self {
        let dev = rt.device();
        let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_voxel_mesher_bgl"),
            entries: &[
                // params
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // padded voxels (u32 per cell)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // header (atomic counter)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // output faces
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let code = r#"
struct Params { padded_dim: u32, chunk_dim: u32, max_out: u32, _pad0: u32 };
struct OutHeader { count: atomic<u32>, _pad: vec3<u32> };
struct FaceInstance {
  x: i32, y: i32, z: i32, dir: u32,
  pi: u32, _pad0: u32, _pad1: u32, _pad2: u32
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> vox: array<u32>;
@group(0) @binding(2) var<storage, read_write> h: OutHeader;
@group(0) @binding(3) var<storage, read_write> out_faces: array<FaceInstance>;

fn idx(x: i32, y: i32, z: i32) -> u32 {
  let d = i32(p.padded_dim);
  return u32(z * d * d + y * d + x);
}

fn load_pi(x: i32, y: i32, z: i32) -> u32 {
  return vox[idx(x, y, z)];
}

fn emit_face(x: i32, y: i32, z: i32, dir: u32, pi: u32) {
  let i = atomicAdd(&h.count, 1u);
  if (i >= p.max_out) { return; }
  out_faces[i] = FaceInstance(x, y, z, dir, pi, 0u, 0u, 0u);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let d = i32(p.chunk_dim);
  let n = u32(d * d * d);
  let i = gid.x;
  if (i >= n) { return; }

  let ix = i % u32(d);
  let iy = (i / u32(d)) % u32(d);
  let iz = i / (u32(d) * u32(d));
  // interior coords in padded grid
  let x = i32(ix) + 1;
  let y = i32(iy) + 1;
  let z = i32(iz) + 1;

  let pi = load_pi(x, y, z);
  if (pi == 0u) { return; }

  // +X / -X
  if (load_pi(x + 1, y, z) == 0u) { emit_face(x, y, z, 0u, pi); }
  if (load_pi(x - 1, y, z) == 0u) { emit_face(x, y, z, 1u, pi); }
  // +Y / -Y
  if (load_pi(x, y + 1, z) == 0u) { emit_face(x, y, z, 2u, pi); }
  if (load_pi(x, y - 1, z) == 0u) { emit_face(x, y, z, 3u, pi); }
  // +Z / -Z
  if (load_pi(x, y, z + 1) == 0u) { emit_face(x, y, z, 4u, pi); }
  if (load_pi(x, y, z - 1) == 0u) { emit_face(x, y, z, 5u, pi); }
}
"#;

        let sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c3d_voxel_mesher_wgsl"),
            source: wgpu::ShaderSource::Wgsl(code.into()),
        });
        let pl = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("c3d_voxel_mesher_pl"),
            bind_group_layouts: &[&bgl],
            immediate_size: 0,
        });
        let ppl = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_voxel_mesher_ppl"),
            layout: Some(&pl),
            module: &sm,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { bgl, ppl, scratch: Mutex::new(GpuVoxelMesherScratch::default()) }
    }

    /// Build a padded voxel buffer for a given chunk key from CPU chunk maps.
    ///
    /// - `chunks`: map chunk_key -> dense u8[CHUNK_SIZE^3]
    /// - `ck`: center chunk key
    ///
    /// Returns a Vec<u32> of size PADDED^3, where interior [1..=CHUNK_SIZE] is the chunk.
    pub fn build_padded_u32(
        chunks: &std::collections::HashMap<IVec3, Vec<u8>>,
        ck: IVec3,
    ) -> Vec<u32> {
        let d = PADDED as usize;
        let mut out = vec![0u32; d * d * d];
        let cs = CHUNK_SIZE as usize;
        let stride_y = d;
        let stride_z = d * d;

        for z in 0..PADDED {
            for y in 0..PADDED {
                for x in 0..PADDED {
                    let gx = ck.x * CHUNK_SIZE + (x - 1);
                    let gy = ck.y * CHUNK_SIZE + (y - 1);
                    let gz = ck.z * CHUNK_SIZE + (z - 1);
                    let pi = get_pi_from_chunks(chunks, IVec3::new(gx, gy, gz));
                    let idx = (z as usize) * stride_z + (y as usize) * stride_y + (x as usize);
                    out[idx] = pi as u32;
                }
            }
        }
        out
    }

    /// Generate face instances for a padded chunk buffer.
    /// Returns `(count, faces)`.
    pub fn mesh_faces(
        &self,
        rt: &GpuRuntime,
        padded_voxels_u32: &[u32],
        max_out: u32,
    ) -> Vec<FaceInstance> {
        let dev = rt.device();
        let q = rt.queue();
        let mut s = self.scratch.lock().unwrap();
        let vox_len_u32 = padded_voxels_u32.len();
        let vox_size = (vox_len_u32 * std::mem::size_of::<u32>()) as u64;
        let faces_size = (std::mem::size_of::<FaceInstance>() as u64) * (max_out as u64);
        ensure_buf(dev, &mut s.param, "c3d_voxel_mesher_params", std::mem::size_of::<Params>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        ensure_buf(dev, &mut s.vox, "c3d_voxel_mesher_voxels", vox_size, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        ensure_buf(dev, &mut s.header, "c3d_voxel_mesher_header", std::mem::size_of::<OutHeader>() as u64, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST);
        ensure_buf(dev, &mut s.faces, "c3d_voxel_mesher_faces", faces_size, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST);
        ensure_buf(dev, &mut s.read_header, "c3d_voxel_mesher_read_header", std::mem::size_of::<OutHeader>() as u64, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
        ensure_buf(dev, &mut s.read_faces, "c3d_voxel_mesher_read_faces", faces_size, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
        let need_bg = s.bg.is_none() || s.vox_len_u32 != vox_len_u32 || s.max_out != max_out;
        if need_bg {
            let (param_buf, vox_buf, header_buf, faces_buf) = (
                s.param.as_ref().unwrap(),
                s.vox.as_ref().unwrap(),
                s.header.as_ref().unwrap(),
                s.faces.as_ref().unwrap(),
            );
            let new_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("c3d_voxel_mesher_bg"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: param_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: vox_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: header_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: faces_buf.as_entire_binding() },
                ],
            });
            s.bg = Some(new_bg);
            s.vox_len_u32 = vox_len_u32;
            s.max_out = max_out;
        }
        let (param_buf, vox_buf, header_buf, faces_buf, read_header, read_faces) = (
            s.param.as_ref().unwrap(),
            s.vox.as_ref().unwrap(),
            s.header.as_ref().unwrap(),
            s.faces.as_ref().unwrap(),
            s.read_header.as_ref().unwrap(),
            s.read_faces.as_ref().unwrap(),
        );

        let p = Params {
            padded_dim: PADDED as u32,
            chunk_dim: CHUNK_SIZE as u32,
            max_out,
            _pad0: 0,
        };
        let header_init = OutHeader { count: 0, _pad0: [0; 3], _pad1: [0; 4] };
        q.write_buffer(param_buf, 0, bytemuck::bytes_of(&p));
        q.write_buffer(vox_buf, 0, bytemuck::cast_slice(padded_voxels_u32));
        q.write_buffer(header_buf, 0, bytemuck::bytes_of(&header_init));
        let bg = s.bg.as_ref().unwrap();

        let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("c3d_voxel_mesher_enc"),
        });
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("c3d_voxel_mesher_pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.ppl);
            cpass.set_bind_group(0, bg, &[]);
            let n = (CHUNK_SIZE as u32) * (CHUNK_SIZE as u32) * (CHUNK_SIZE as u32);
            let wg = (n + 255) / 256;
            cpass.dispatch_workgroups(wg, 1, 1);
        }
        enc.copy_buffer_to_buffer(&header_buf, 0, &read_header, 0, std::mem::size_of::<OutHeader>() as u64);
        enc.copy_buffer_to_buffer(&faces_buf, 0, &read_faces, 0, (std::mem::size_of::<FaceInstance>() as u64) * (max_out as u64));
        q.submit(Some(enc.finish()));

        // Readback (blocking, debug/prototype).
        map_read_blocking(dev, read_header);
        let hdr = {
            let v = read_header.slice(..).get_mapped_range();
            let h: OutHeader = *bytemuck::from_bytes(&v);
            drop(v);
            read_header.unmap();
            h
        };
        let cnt = (hdr.count as usize).min(max_out as usize);

        map_read_blocking(dev, read_faces);
        let faces: Vec<FaceInstance> = {
            let v = read_faces.slice(..).get_mapped_range();
            let all: &[FaceInstance] = bytemuck::cast_slice(&v);
            let out = all.get(0..cnt).unwrap_or(&[]).to_vec();
            drop(v);
            read_faces.unmap();
            out
        };
        faces
    }

    /// Full MVP: CPU chunks -> padded upload -> GPU faces -> CPU geometry.
    pub fn mesh_chunk_to_geometry(
        &self,
        rt: &GpuRuntime,
        chunks: &std::collections::HashMap<IVec3, Vec<u8>>,
        ck: IVec3,
        palette: &[vox::PaletteEntry],
        voxel_size: f32,
        max_out: u32,
    ) -> Geometry {
        let padded = Self::build_padded_u32(chunks, ck);
        let faces = self.mesh_faces(rt, &padded, max_out);
        faces_to_geometry(&faces, ck, palette, voxel_size)
    }
}

#[inline]
fn map_read_blocking(dev: &wgpu::Device, buf: &wgpu::Buffer) {
    // wgpu 28: map_async is callback-based; poll() drives completion.
    let slice = buf.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = dev.poll(wgpu::PollType::wait_indefinitely());
}

#[inline]
fn chunk_coord(p: IVec3) -> IVec3 {
    IVec3::new(
        p.x.div_euclid(CHUNK_SIZE),
        p.y.div_euclid(CHUNK_SIZE),
        p.z.div_euclid(CHUNK_SIZE),
    )
}

#[inline]
fn chunk_local(p: IVec3) -> IVec3 {
    IVec3::new(
        p.x.rem_euclid(CHUNK_SIZE),
        p.y.rem_euclid(CHUNK_SIZE),
        p.z.rem_euclid(CHUNK_SIZE),
    )
}

#[inline]
fn chunk_idx(lp: IVec3) -> usize {
    let cs = CHUNK_SIZE as usize;
    (lp.z as usize) * cs * cs + (lp.y as usize) * cs + (lp.x as usize)
}

#[inline]
fn get_pi_from_chunks(chunks: &std::collections::HashMap<IVec3, Vec<u8>>, p: IVec3) -> u8 {
    let ck = chunk_coord(p);
    let lp = chunk_local(p);
    chunks
        .get(&ck)
        .and_then(|v| v.get(chunk_idx(lp)).copied())
        .unwrap_or(0)
}

#[inline]
fn cd_from_pi(palette: &[vox::PaletteEntry], pi: u32) -> Vec3 {
    let c = palette.get(pi as usize).map(|p| p.color).unwrap_or([255, 255, 255, 255]);
    Vec3::new(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0)
}

#[inline]
fn point_cached(
    geo: &mut Geometry,
    cache: &mut std::collections::HashMap<IVec3, PointId>,
    ps: &mut Vec<Vec3>,
    p: IVec3,
    vs: f32,
) -> PointId {
    if let Some(id) = cache.get(&p).copied() { return id; }
    let id = geo.add_point();
    cache.insert(p, id);
    ps.push(Vec3::new(p.x as f32 * vs, p.y as f32 * vs, p.z as f32 * vs));
    id
}

#[inline]
fn emit_quad_cached(
    geo: &mut Geometry,
    cache: &mut std::collections::HashMap<IVec3, PointId>,
    ps: &mut Vec<Vec3>,
    prim_cd: &mut Vec<Vec3>,
    cd: Vec3,
    p0: IVec3,
    p1: IVec3,
    p2: IVec3,
    p3: IVec3,
    vs: f32,
) {
    let a = point_cached(geo, cache, ps, p0, vs);
    let b = point_cached(geo, cache, ps, p1, vs);
    let c = point_cached(geo, cache, ps, p2, vs);
    let d = point_cached(geo, cache, ps, p3, vs);
    let v0 = geo.add_vertex(a);
    let v1 = geo.add_vertex(b);
    let v2 = geo.add_vertex(c);
    let v3 = geo.add_vertex(d);
    geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: vec![v0, v1, v2, v3] }));
    prim_cd.push(cd);
}

/// Convert face instances of a single chunk into `Geometry`.
///
/// Notes:
/// - This emits 1 quad primitive per face (no greedy merge yet).
/// - Face `x/y/z` are in *padded* coords [1..=CHUNK_SIZE] for the voxel cell.
pub fn faces_to_geometry(
    faces: &[FaceInstance],
    ck: IVec3,
    palette: &[vox::PaletteEntry],
    voxel_size: f32,
) -> Geometry {
    let vs = voxel_size.max(0.001);
    let mut out = Geometry::new();
    if faces.is_empty() { return out; }

    let base = ck * CHUNK_SIZE;
    let mut ps: Vec<Vec3> = Vec::new();
    let mut cds_prim: Vec<Vec3> = Vec::with_capacity(faces.len());
    let mut ns_prim: Vec<Vec3> = Vec::with_capacity(faces.len());
    let mut ns_vert: Vec<Vec3> = Vec::with_capacity(faces.len() * 4);
    let mut chunk_prim: Vec<IVec3> = Vec::with_capacity(faces.len());
    let mut point_cache: std::collections::HashMap<IVec3, PointId> = std::collections::HashMap::new();

    for f in faces {
        let lx = f.x - 1;
        let ly = f.y - 1;
        let lz = f.z - 1;
        let cd = cd_from_pi(palette, f.pi);

        match f.dir {
            // +X
            0 => {
                let x = base.x + lx + 1;
                let y0 = base.y + ly; let y1 = base.y + ly + 1;
                let z0 = base.z + lz; let z1 = base.z + lz + 1;
                emit_quad_cached(&mut out, &mut point_cache, &mut ps, &mut cds_prim, cd,
                    IVec3::new(x, y0, z0),
                    IVec3::new(x, y1, z0),
                    IVec3::new(x, y1, z1),
                    IVec3::new(x, y0, z1),
                    vs
                );
                let n = Vec3::X;
                ns_prim.push(n);
                ns_vert.extend_from_slice(&[n, n, n, n]);
            }
            // -X
            1 => {
                let x = base.x + lx;
                let y0 = base.y + ly; let y1 = base.y + ly + 1;
                let z0 = base.z + lz; let z1 = base.z + lz + 1;
                emit_quad_cached(&mut out, &mut point_cache, &mut ps, &mut cds_prim, cd,
                    IVec3::new(x, y0, z0),
                    IVec3::new(x, y0, z1),
                    IVec3::new(x, y1, z1),
                    IVec3::new(x, y1, z0),
                    vs
                );
                let n = -Vec3::X;
                ns_prim.push(n);
                ns_vert.extend_from_slice(&[n, n, n, n]);
            }
            // +Y
            2 => {
                let y = base.y + ly + 1;
                let x0 = base.x + lx; let x1 = base.x + lx + 1;
                let z0 = base.z + lz; let z1 = base.z + lz + 1;
                emit_quad_cached(&mut out, &mut point_cache, &mut ps, &mut cds_prim, cd,
                    IVec3::new(x0, y, z0),
                    IVec3::new(x0, y, z1),
                    IVec3::new(x1, y, z1),
                    IVec3::new(x1, y, z0),
                    vs
                );
                let n = Vec3::Y;
                ns_prim.push(n);
                ns_vert.extend_from_slice(&[n, n, n, n]);
            }
            // -Y
            3 => {
                let y = base.y + ly;
                let x0 = base.x + lx; let x1 = base.x + lx + 1;
                let z0 = base.z + lz; let z1 = base.z + lz + 1;
                emit_quad_cached(&mut out, &mut point_cache, &mut ps, &mut cds_prim, cd,
                    IVec3::new(x0, y, z0),
                    IVec3::new(x1, y, z0),
                    IVec3::new(x1, y, z1),
                    IVec3::new(x0, y, z1),
                    vs
                );
                let n = -Vec3::Y;
                ns_prim.push(n);
                ns_vert.extend_from_slice(&[n, n, n, n]);
            }
            // +Z
            4 => {
                let z = base.z + lz + 1;
                let x0 = base.x + lx; let x1 = base.x + lx + 1;
                let y0 = base.y + ly; let y1 = base.y + ly + 1;
                emit_quad_cached(&mut out, &mut point_cache, &mut ps, &mut cds_prim, cd,
                    IVec3::new(x0, y0, z),
                    IVec3::new(x1, y0, z),
                    IVec3::new(x1, y1, z),
                    IVec3::new(x0, y1, z),
                    vs
                );
                let n = Vec3::Z;
                ns_prim.push(n);
                ns_vert.extend_from_slice(&[n, n, n, n]);
            }
            // -Z
            5 => {
                let z = base.z + lz;
                let x0 = base.x + lx; let x1 = base.x + lx + 1;
                let y0 = base.y + ly; let y1 = base.y + ly + 1;
                emit_quad_cached(&mut out, &mut point_cache, &mut ps, &mut cds_prim, cd,
                    IVec3::new(x0, y0, z),
                    IVec3::new(x0, y1, z),
                    IVec3::new(x1, y1, z),
                    IVec3::new(x1, y0, z),
                    vs
                );
                let n = -Vec3::Z;
                ns_prim.push(n);
                ns_vert.extend_from_slice(&[n, n, n, n]);
            }
            _ => continue,
        }

        chunk_prim.push(ck);
    }

    if !ps.is_empty() { out.insert_point_attribute(attrs::P, Attribute::new(ps)); }
    if !cds_prim.is_empty() { out.insert_primitive_attribute(attrs::CD, Attribute::new(cds_prim)); }
    if !ns_prim.is_empty() { out.insert_primitive_attribute(attrs::N, Attribute::new(ns_prim)); }
    if !ns_vert.is_empty() { out.insert_vertex_attribute(attrs::N, Attribute::new(ns_vert)); }
    if !chunk_prim.is_empty() { out.insert_primitive_attribute("__voxel_chunk", Attribute::new(chunk_prim)); }
    out
}

