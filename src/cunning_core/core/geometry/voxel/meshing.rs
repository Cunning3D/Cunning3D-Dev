use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use super::dirty::chunk_coord;
use super::types::{PaletteEntry, VoxelPi, CHUNK_SIZE};
use super::volume::{SdfChunkKey, VoxelVolume};

use crate::libs::geometry::attrs;
use crate::libs::geometry::mesh::{Attribute, GeoPrimitive, Geometry, PolygonPrim};
use crate::libs::geometry::ids::PointId;

/// Simple CPU mesh buffers for a chunk.
#[derive(Debug, Clone, Default)]
pub struct ChunkMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

#[inline]
fn cd_from_pi(palette: &[PaletteEntry], pi: VoxelPi) -> [f32; 4] {
    let c = palette.get(pi as usize).map(|p| p.color).unwrap_or([255, 255, 255, 255]);
    [
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
        c[3] as f32 / 255.0,
    ]
}

#[inline]
fn greedy_mesh_mask(mask: &mut [i32], w: usize, h: usize, mut emit: impl FnMut(usize, usize, usize, usize, i32)) {
    for y in 0..h {
        let mut x = 0usize;
        while x < w {
            let v = mask[y * w + x];
            if v == 0 { x += 1; continue; }
            let mut ww = 1usize;
            while x + ww < w && mask[y * w + x + ww] == v { ww += 1; }
            let mut hh = 1usize;
            'outer: while y + hh < h {
                for xx in 0..ww {
                    if mask[(y + hh) * w + x + xx] != v { break 'outer; }
                }
                hh += 1;
            }
            emit(x, y, ww, hh, v);
            for yy in 0..hh { for xx in 0..ww { mask[(y + yy) * w + x + xx] = 0; } }
            x += ww;
        }
    }
}

#[inline]
fn chunk_local(p: IVec3) -> IVec3 {
    IVec3::new(p.x.rem_euclid(CHUNK_SIZE), p.y.rem_euclid(CHUNK_SIZE), p.z.rem_euclid(CHUNK_SIZE))
}

#[inline]
fn chunk_idx(lp: IVec3) -> usize {
    let cs = CHUNK_SIZE as usize;
    (lp.z as usize) * cs * cs + (lp.y as usize) * cs + (lp.x as usize)
}

#[inline]
fn get_pi(chunks: &HashMap<IVec3, Vec<u8>>, p: IVec3) -> u8 {
    let ck = chunk_coord(p);
    let lp = chunk_local(p);
    chunks.get(&ck).and_then(|v| v.get(chunk_idx(lp)).copied()).unwrap_or(0)
}

/// Greedy mesh a single chunk (requires neighbor sampling, so we query `volume.get_pi`).
pub fn mesh_chunk_greedy(volume: &VoxelVolume, ck: SdfChunkKey) -> Option<ChunkMesh> {
    let chunk = volume.chunks.get(&ck)?;
    if chunk.solid_count == 0 { return None; }
    // Build a temporary view of nearby chunks to avoid per-voxel HashMap lookups.
    // (This is a baseline; later we can add a structured cache.)
    let mut chunks: HashMap<IVec3, Vec<u8>> = HashMap::new();
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let k = ck + IVec3::new(dx, dy, dz);
                if let Some(c) = volume.chunks.get(&k) {
                    chunks.insert(k, c.data.clone());
                }
            }
        }
    }
    // Ensure center chunk exists
    chunks.entry(ck).or_insert_with(|| chunk.data.clone());

    let cs = CHUNK_SIZE.max(4);
    let mut mask: Vec<i32> = vec![0; (cs as usize) * (cs as usize)];
    let base = ck * cs;
    let pal = &volume.palette;

    let mut out = ChunkMesh::default();
    // We build as unindexed quads then index with two triangles.
    // Keep it simple for now.
    for axis in 0..3 {
        for s in 0..=cs {
            mask.fill(0);
            for v in 0..cs {
                for u in 0..cs {
                    let (a, b) = match axis {
                        0 => (get_pi(&chunks, base + IVec3::new(s - 1, u, v)), get_pi(&chunks, base + IVec3::new(s, u, v))),
                        1 => (get_pi(&chunks, base + IVec3::new(u, s - 1, v)), get_pi(&chunks, base + IVec3::new(u, s, v))),
                        _ => (get_pi(&chunks, base + IVec3::new(u, v, s - 1)), get_pi(&chunks, base + IVec3::new(u, v, s))),
                    };
                    let val = if a != 0 && b == 0 { a as i32 } else if a == 0 && b != 0 { -(b as i32) } else { 0 };
                    mask[(v as usize) * (cs as usize) + (u as usize)] = val;
                }
            }

            greedy_mesh_mask(&mut mask, cs as usize, cs as usize, |u0, v0, uw, vh, val| {
                let pi = val.unsigned_abs() as u8;
                let col = cd_from_pi(pal, pi);
                let u1 = u0 as i32 + uw as i32;
                let v1 = v0 as i32 + vh as i32;
                let u0 = u0 as i32;
                let v0 = v0 as i32;
                let s = s as i32;
                let n = match axis {
                    0 => Vec3::X,
                    1 => Vec3::Y,
                    _ => Vec3::Z,
                } * if val > 0 { 1.0 } else { -1.0 };

                let vs = volume.voxel_size;
                let mut p = |x: i32, y: i32, z: i32| -> [f32; 3] {
                    [(x as f32) * vs, (y as f32) * vs, (z as f32) * vs]
                };

                let (p0, p1, p2, p3) = match axis {
                    0 => {
                        let x = base.x + s;
                        let y0 = base.y + u0; let y1 = base.y + u1;
                        let z0 = base.z + v0; let z1 = base.z + v1;
                        if val > 0 {
                            (p(x, y0, z0), p(x, y1, z0), p(x, y1, z1), p(x, y0, z1))
                        } else {
                            (p(x, y0, z0), p(x, y0, z1), p(x, y1, z1), p(x, y1, z0))
                        }
                    }
                    1 => {
                        let y = base.y + s;
                        let x0 = base.x + u0; let x1 = base.x + u1;
                        let z0 = base.z + v0; let z1 = base.z + v1;
                        if val > 0 {
                            (p(x0, y, z0), p(x0, y, z1), p(x1, y, z1), p(x1, y, z0))
                        } else {
                            (p(x0, y, z0), p(x1, y, z0), p(x1, y, z1), p(x0, y, z1))
                        }
                    }
                    _ => {
                        let z = base.z + s;
                        let x0 = base.x + u0; let x1 = base.x + u1;
                        let y0 = base.y + v0; let y1 = base.y + v1;
                        if val > 0 {
                            (p(x0, y0, z), p(x1, y0, z), p(x1, y1, z), p(x0, y1, z))
                        } else {
                            (p(x0, y0, z), p(x0, y1, z), p(x1, y1, z), p(x1, y0, z))
                        }
                    }
                };

                let base_i = out.positions.len() as u32;
                out.positions.extend_from_slice(&[p0, p1, p2, p3]);
                let nn = [n.x, n.y, n.z];
                out.normals.extend_from_slice(&[nn, nn, nn, nn]);
                out.colors.extend_from_slice(&[col, col, col, col]);
                out.indices.extend_from_slice(&[
                    base_i, base_i + 1, base_i + 2,
                    base_i, base_i + 2, base_i + 3,
                ]);
            });
        }
    }

    Some(out)
}

pub fn mesh_dirty_chunks_greedy(volume: &VoxelVolume, dirty: &HashSet<IVec3>) -> HashMap<IVec3, ChunkMesh> {
    let mut out: HashMap<IVec3, ChunkMesh> = HashMap::new();
    for &ck in dirty {
        if let Some(m) = mesh_chunk_greedy(volume, ck) {
            out.insert(ck, m);
        } else {
            out.remove(&ck);
        }
    }
    out
}

/// Convenience: materialize an entire volume to `Geometry` with greedy meshing.
pub fn volume_to_geometry_greedy(volume: &VoxelVolume, filter_chunks: Option<&HashSet<IVec3>>) -> Geometry {
    let mut out = Geometry::new();
    if volume.chunks.is_empty() { return out; }
    let mut ps: Vec<Vec3> = Vec::new();
    let mut cds_prim: Vec<Vec3> = Vec::new();
    let mut ns_prim: Vec<Vec3> = Vec::new();
    let mut ns_vert: Vec<Vec3> = Vec::new();

    let mut point_cache: HashMap<IVec3, PointId> = HashMap::new();
    let cs = CHUNK_SIZE.max(4);
    let mut mask: Vec<i32> = vec![0; (cs as usize) * (cs as usize)];

    // Build a cheap map of chunk -> data slice reference (clone-free).
    let chunks: HashMap<IVec3, &Vec<u8>> = volume.chunks.iter().map(|(k, c)| (*k, &c.data)).collect();

    for (&ck, _) in chunks.iter() {
        if let Some(f) = filter_chunks {
            if !f.contains(&ck) { continue; }
        }
        let base = ck * cs;
        for axis in 0..3 {
            for s in 0..=cs {
                mask.fill(0);
                for v in 0..cs {
                    for u in 0..cs {
                        let (a, b) = match axis {
                            0 => (get_pi_ref(&chunks, base + IVec3::new(s - 1, u, v)), get_pi_ref(&chunks, base + IVec3::new(s, u, v))),
                            1 => (get_pi_ref(&chunks, base + IVec3::new(u, s - 1, v)), get_pi_ref(&chunks, base + IVec3::new(u, s, v))),
                            _ => (get_pi_ref(&chunks, base + IVec3::new(u, v, s - 1)), get_pi_ref(&chunks, base + IVec3::new(u, v, s))),
                        };
                        let val = if a != 0 && b == 0 { a as i32 } else if a == 0 && b != 0 { -(b as i32) } else { 0 };
                        mask[(v as usize) * (cs as usize) + (u as usize)] = val;
                    }
                }

                greedy_mesh_mask(&mut mask, cs as usize, cs as usize, |u0, v0, uw, vh, val| {
                    let pi = val.unsigned_abs() as u8;
                    let c = volume.palette.get(pi as usize).map(|p| p.color).unwrap_or([255, 255, 255, 255]);
                    let cd = Vec3::new(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0);
                    let u1 = u0 as i32 + uw as i32;
                    let v1 = v0 as i32 + vh as i32;
                    let u0 = u0 as i32;
                    let v0 = v0 as i32;
                    let s = s as i32;
                    let n = match axis { 0 => Vec3::X, 1 => Vec3::Y, _ => Vec3::Z } * if val > 0 { 1.0 } else { -1.0 };

                    let quad = match axis {
                        0 => {
                            let x = base.x + s;
                            let y0 = base.y + u0; let y1 = base.y + u1;
                            let z0 = base.z + v0; let z1 = base.z + v1;
                            if val > 0 { [IVec3::new(x, y0, z0), IVec3::new(x, y1, z0), IVec3::new(x, y1, z1), IVec3::new(x, y0, z1)] }
                            else { [IVec3::new(x, y0, z0), IVec3::new(x, y0, z1), IVec3::new(x, y1, z1), IVec3::new(x, y1, z0)] }
                        }
                        1 => {
                            let y = base.y + s;
                            let x0 = base.x + u0; let x1 = base.x + u1;
                            let z0 = base.z + v0; let z1 = base.z + v1;
                            if val > 0 { [IVec3::new(x0, y, z0), IVec3::new(x0, y, z1), IVec3::new(x1, y, z1), IVec3::new(x1, y, z0)] }
                            else { [IVec3::new(x0, y, z0), IVec3::new(x1, y, z0), IVec3::new(x1, y, z1), IVec3::new(x0, y, z1)] }
                        }
                        _ => {
                            let z = base.z + s;
                            let x0 = base.x + u0; let x1 = base.x + u1;
                            let y0 = base.y + v0; let y1 = base.y + v1;
                            if val > 0 { [IVec3::new(x0, y0, z), IVec3::new(x1, y0, z), IVec3::new(x1, y1, z), IVec3::new(x0, y1, z)] }
                            else { [IVec3::new(x0, y0, z), IVec3::new(x0, y1, z), IVec3::new(x1, y1, z), IVec3::new(x1, y0, z)] }
                        }
                    };

                    // Emit as unique points (cached by voxel-grid corner).
                    let vs = volume.voxel_size;
                    let mut vids = Vec::with_capacity(4);
                    for &p in quad.iter() {
                        let pid = *point_cache.entry(p).or_insert_with(|| {
                            let id = out.add_point();
                            ps.push(Vec3::new(p.x as f32 * vs, p.y as f32 * vs, p.z as f32 * vs));
                            id
                        });
                        vids.push(out.add_vertex(pid));
                        ns_vert.push(n);
                    }
                    out.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: vids }));
                    cds_prim.push(cd);
                    ns_prim.push(n);
                });
            }
        }
    }

    if !ps.is_empty() { out.insert_point_attribute(attrs::P, Attribute::new(ps)); }
    if !cds_prim.is_empty() { out.insert_primitive_attribute(attrs::CD, Attribute::new(cds_prim)); }
    if !ns_prim.is_empty() { out.insert_primitive_attribute(attrs::N, Attribute::new(ns_prim)); }
    if !ns_vert.is_empty() { out.insert_vertex_attribute(attrs::N, Attribute::new(ns_vert)); }
    out
}

#[inline]
fn get_pi_ref(chunks: &HashMap<IVec3, &Vec<u8>>, p: IVec3) -> u8 {
    let ck = chunk_coord(p);
    let lp = IVec3::new(p.x.rem_euclid(CHUNK_SIZE), p.y.rem_euclid(CHUNK_SIZE), p.z.rem_euclid(CHUNK_SIZE));
    let cs = CHUNK_SIZE as usize;
    let idx = (lp.z as usize) * cs * cs + (lp.y as usize) * cs + (lp.x as usize);
    chunks.get(&ck).and_then(|v| v.get(idx).copied()).unwrap_or(0)
}

