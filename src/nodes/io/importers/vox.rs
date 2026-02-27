use super::FileImporter;
use crate::libs::geometry::mesh::Geometry;
use bevy::prelude::IVec3;
use cunning_kernel::algorithms::algorithms_editor::voxel as vox;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use uuid::Uuid;

const ATTR_VOXEL_SIZE_DETAIL: &str = "__voxel_size";

#[inline]
fn hash64(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

pub struct VoxImporter;

impl FileImporter for VoxImporter {
    fn extensions(&self) -> &[&str] { &["vox"] }

    fn import(&self, path: &Path) -> Result<Geometry, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("VOX read error: {}", e))?;
        let voxel_size = 0.1f32;
        let path_s = path.to_string_lossy();
        let h0 = hash64(path_s.as_ref()) as u128;
        let h1 = hash64(&format!("vox:{}", path_s)) as u128;
        let node_id = Uuid::from_u128((h0 << 64) | h1);
        import_vox_bytes(&bytes, node_id, voxel_size)
    }
}

pub(crate) fn import_vox_bytes(bytes: &[u8], node_id: Uuid, voxel_size: f32) -> Result<Geometry, String> {
    let vox = parse_vox(bytes).map_err(|e| format!("VOX parse error: {}", e))?;
    let mut mn = IVec3::splat(i32::MAX);
    for (p, _) in vox.voxels.iter() { mn = mn.min(*p); }
    if mn.x == i32::MAX { mn = IVec3::ZERO; }
    let mut grid = vox::DiscreteSdfGrid::new(voxel_size);
    for (i, e) in vox.palette.into_iter().enumerate() { if i < grid.palette.len() { grid.palette[i] = e; } }
    for (p, pi) in vox.voxels.into_iter() {
        let pi = if let Some(imap) = vox.imap.as_ref() { imap.get(pi as usize).copied().unwrap_or(pi) } else { pi }.max(1);
        let p = p - mn;
        grid.set(p.x, p.y, p.z, vox::DiscreteVoxel { palette_index: pi, color_override: None });
    }
    cunning_kernel::nodes::voxel::voxel_edit::voxel_render_register_grid(node_id, voxel_size, grid);
    let mut geo = Geometry::new();
    geo.set_detail_attribute(ATTR_VOXEL_SIZE_DETAIL, vec![voxel_size]);
    geo.set_detail_attribute("__voxel_pure", vec![1.0f32]);
    geo.set_detail_attribute("__voxel_node", vec![node_id.to_string()]);
    Ok(geo)
}

#[derive(Default)]
struct VoxData {
    voxels: Vec<(IVec3, u8)>,
    palette: Vec<vox::discrete::PaletteEntry>,
    imap: Option<[u8; 256]>,
}

#[derive(Clone, Default)]
struct Model { voxels: Vec<(IVec3, u8)> }

#[derive(Clone, Default)]
struct Trn { id: i32, attrs: HashMap<String, String>, child: i32, frames: Vec<HashMap<String, String>> }
#[derive(Clone, Default)]
struct Grp { id: i32, children: Vec<i32> }
#[derive(Clone, Default)]
struct Shp { id: i32, models: Vec<i32> }

#[derive(Clone, Copy, Default)]
struct Xform { m: [[i32; 3]; 3], t: [i32; 3] }

impl Xform {
    fn identity() -> Self { Self { m: [[1, 0, 0], [0, 1, 0], [0, 0, 1]], t: [0, 0, 0] } }
    fn apply(&self, p: IVec3) -> IVec3 {
        let x = self.m[0][0] * p.x + self.m[0][1] * p.y + self.m[0][2] * p.z + self.t[0];
        let y = self.m[1][0] * p.x + self.m[1][1] * p.y + self.m[1][2] * p.z + self.t[1];
        let z = self.m[2][0] * p.x + self.m[2][1] * p.y + self.m[2][2] * p.z + self.t[2];
        IVec3::new(x, y, z)
    }
    fn mul(&self, b: &Xform) -> Xform {
        let mut m = [[0i32; 3]; 3];
        for r in 0..3 { for c in 0..3 { m[r][c] = self.m[r][0] * b.m[0][c] + self.m[r][1] * b.m[1][c] + self.m[r][2] * b.m[2][c]; } }
        let bt = [b.t[0], b.t[1], b.t[2]];
        let at = [self.t[0], self.t[1], self.t[2]];
        let t0 = self.m[0][0] * bt[0] + self.m[0][1] * bt[1] + self.m[0][2] * bt[2] + at[0];
        let t1 = self.m[1][0] * bt[0] + self.m[1][1] * bt[1] + self.m[1][2] * bt[2] + at[1];
        let t2 = self.m[2][0] * bt[0] + self.m[2][1] * bt[1] + self.m[2][2] * bt[2] + at[2];
        Xform { m, t: [t0, t1, t2] }
    }
}

fn parse_vox(bytes: &[u8]) -> Result<VoxData, String> {
    use byteorder::{LittleEndian, ReadBytesExt};
    use std::io::{Cursor, Read};
    let mut cur = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cur.read_exact(&mut magic).map_err(|_| "bad header".to_string())?;
    if &magic != b"VOX " { return Err("missing VOX magic".to_string()); }
    let _ver = cur.read_u32::<LittleEndian>().map_err(|_| "bad version".to_string())?;
    let main = read_chunk(&mut cur, bytes.len()).map_err(|_| "chunk decode failed".to_string())?;
    if &main.id != b"MAIN" { return Err("missing MAIN chunk".to_string()); }

    let mut pack_models: Option<usize> = None;
    let mut models: Vec<Model> = Vec::new();
    let mut pending_size: Option<IVec3> = None;

    let mut palette: Vec<vox::discrete::PaletteEntry> = (0..256u32).map(|i| {
        let v = (i as u8).max(1);
        vox::discrete::PaletteEntry { color: [v, v, v, 255], ..Default::default() }
    }).collect();
    palette[0] = vox::discrete::PaletteEntry { color: [0, 0, 0, 0], ..Default::default() };
    let mut imap: Option<[u8; 256]> = None;

    let mut trns: HashMap<i32, Trn> = HashMap::new();
    let mut grps: HashMap<i32, Grp> = HashMap::new();
    let mut shps: HashMap<i32, Shp> = HashMap::new();
    let mut referenced: HashSet<i32> = HashSet::new();

    for ch in main.children.iter() {
        match &ch.id {
            b"PACK" => {
                let mut c = Cursor::new(ch.content.as_slice());
                pack_models = Some(c.read_u32::<LittleEndian>().map_err(|_| "bad PACK".to_string())? as usize);
            }
            b"SIZE" => {
                let mut c = Cursor::new(ch.content.as_slice());
                let sx = c.read_i32::<LittleEndian>().map_err(|_| "bad SIZE".to_string())?;
                let sy = c.read_i32::<LittleEndian>().map_err(|_| "bad SIZE".to_string())?;
                let sz = c.read_i32::<LittleEndian>().map_err(|_| "bad SIZE".to_string())?;
                let _ = (sx, sy, sz);
                pending_size = Some(IVec3::new(sx, sy, sz));
            }
            b"XYZI" => {
                let mut c = Cursor::new(ch.content.as_slice());
                let n = c.read_u32::<LittleEndian>().map_err(|_| "bad XYZI".to_string())? as usize;
                let mut out: Vec<(IVec3, u8)> = Vec::with_capacity(n);
                for _ in 0..n {
                    let x = c.read_u8().map_err(|_| "bad XYZI".to_string())? as i32;
                    let y = c.read_u8().map_err(|_| "bad XYZI".to_string())? as i32;
                    let z = c.read_u8().map_err(|_| "bad XYZI".to_string())? as i32;
                    let i = c.read_u8().map_err(|_| "bad XYZI".to_string())?;
                    out.push((IVec3::new(x, y, z), i.max(1)));
                }
                let _ = pending_size.take();
                models.push(Model { voxels: out });
            }
            b"RGBA" => {
                let mut c = Cursor::new(ch.content.as_slice());
                for i in 1..=255usize {
                    let r = c.read_u8().map_err(|_| "bad RGBA".to_string())?;
                    let g = c.read_u8().map_err(|_| "bad RGBA".to_string())?;
                    let b = c.read_u8().map_err(|_| "bad RGBA".to_string())?;
                    let a = c.read_u8().map_err(|_| "bad RGBA".to_string())?;
                    palette[i] = vox::discrete::PaletteEntry { color: [r, g, b, a], ..Default::default() };
                }
            }
            b"IMAP" => {
                let mut c = Cursor::new(ch.content.as_slice());
                let mut arr = [0u8; 256];
                for i in 0..256usize {
                    let v = c.read_i32::<LittleEndian>().map_err(|_| "bad IMAP".to_string())?.clamp(0, 255) as u8;
                    arr[i] = v;
                }
                imap = Some(arr);
            }
            b"nTRN" => {
                let mut c = Cursor::new(ch.content.as_slice());
                let id = c.read_i32::<LittleEndian>().map_err(|_| "bad nTRN".to_string())?;
                let attrs = read_dict(&mut c).map_err(|_| "bad nTRN dict".to_string())?;
                let child = c.read_i32::<LittleEndian>().map_err(|_| "bad nTRN".to_string())?;
                let _reserved = c.read_i32::<LittleEndian>().map_err(|_| "bad nTRN".to_string())?;
                let _layer = c.read_i32::<LittleEndian>().map_err(|_| "bad nTRN".to_string())?;
                let frames_n = c.read_i32::<LittleEndian>().map_err(|_| "bad nTRN".to_string())?.max(0) as usize;
                let mut frames: Vec<HashMap<String, String>> = Vec::with_capacity(frames_n);
                for _ in 0..frames_n { frames.push(read_dict(&mut c).map_err(|_| "bad nTRN frame".to_string())?); }
                referenced.insert(child);
                trns.insert(id, Trn { id, attrs, child, frames });
            }
            b"nGRP" => {
                let mut c = Cursor::new(ch.content.as_slice());
                let id = c.read_i32::<LittleEndian>().map_err(|_| "bad nGRP".to_string())?;
                let _attrs = read_dict(&mut c).map_err(|_| "bad nGRP dict".to_string())?;
                let n = c.read_i32::<LittleEndian>().map_err(|_| "bad nGRP".to_string())?.max(0) as usize;
                let mut children: Vec<i32> = Vec::with_capacity(n);
                for _ in 0..n { let cid = c.read_i32::<LittleEndian>().map_err(|_| "bad nGRP".to_string())?; referenced.insert(cid); children.push(cid); }
                grps.insert(id, Grp { id, children });
            }
            b"nSHP" => {
                let mut c = Cursor::new(ch.content.as_slice());
                let id = c.read_i32::<LittleEndian>().map_err(|_| "bad nSHP".to_string())?;
                let _attrs = read_dict(&mut c).map_err(|_| "bad nSHP dict".to_string())?;
                let n = c.read_i32::<LittleEndian>().map_err(|_| "bad nSHP".to_string())?.max(0) as usize;
                let mut mids: Vec<i32> = Vec::with_capacity(n);
                for _ in 0..n {
                    let mid = c.read_i32::<LittleEndian>().map_err(|_| "bad nSHP".to_string())?;
                    let _mattrs = read_dict(&mut c).map_err(|_| "bad nSHP model dict".to_string())?;
                    mids.push(mid);
                }
                shps.insert(id, Shp { id, models: mids });
            }
            _ => {}
        }
    }

    if models.is_empty() { return Ok(VoxData { voxels: Vec::new(), palette, imap }); }
    if models.is_empty() { return Ok(VoxData { voxels: Vec::new(), palette, imap }); }
    if let Some(expect) = pack_models { let _ = expect; }

    let mut instances: Vec<(usize, Xform)> = Vec::new();
    if trns.is_empty() && grps.is_empty() && shps.is_empty() {
        for mi in 0..models.len() { instances.push((mi, Xform::identity())); }
    } else {
        let mut roots: Vec<i32> = Vec::new();
        for id in trns.keys() { if !referenced.contains(id) { roots.push(*id); } }
        if roots.is_empty() { for id in trns.keys() { roots.push(*id); } }
        for r in roots { visit_node(r, Xform::identity(), &trns, &grps, &shps, &mut instances); }
        if instances.is_empty() { for mi in 0..models.len() { instances.push((mi, Xform::identity())); } }
    }

    let mut out: Vec<(IVec3, u8)> = Vec::new();
    let mut seen: HashSet<(IVec3, u8)> = HashSet::new();
    for (mid, xf) in instances.into_iter() {
        let Some(m) = models.get(mid) else { continue; };
        for (p, pi) in m.voxels.iter().copied() {
            let pw = xf.apply(p);
            let pe = IVec3::new(pw.x, pw.z, pw.y);
            if seen.insert((pe, pi)) { out.push((pe, pi)); }
        }
    }
    Ok(VoxData { voxels: out, palette, imap })
}

#[derive(Clone)]
struct Chunk { id: [u8; 4], content: Vec<u8>, children: Vec<Chunk> }

fn read_chunk(cur: &mut std::io::Cursor<&[u8]>, end: usize) -> Result<Chunk, ()> {
    use byteorder::{LittleEndian, ReadBytesExt};
    use std::io::Read;
    if (cur.position() as usize) + 12 > end { return Err(()); }
    let mut id = [0u8; 4];
    cur.read_exact(&mut id).map_err(|_| ())?;
    let n = cur.read_u32::<LittleEndian>().map_err(|_| ())? as usize;
    let m = cur.read_u32::<LittleEndian>().map_err(|_| ())? as usize;
    if (cur.position() as usize) + n > end { return Err(()); }
    let mut content = vec![0u8; n];
    cur.read_exact(&mut content).map_err(|_| ())?;
    let children_end = (cur.position() as usize).saturating_add(m).min(end);
    let mut children: Vec<Chunk> = Vec::new();
    while (cur.position() as usize) < children_end {
        children.push(read_chunk(cur, children_end)?);
    }
    cur.set_position(children_end as u64);
    Ok(Chunk { id, content, children })
}

fn read_string(cur: &mut std::io::Cursor<&[u8]>) -> Result<String, ()> {
    use byteorder::{LittleEndian, ReadBytesExt};
    use std::io::Read;
    let n = cur.read_i32::<LittleEndian>().map_err(|_| ())?.max(0) as usize;
    let mut buf = vec![0u8; n];
    cur.read_exact(&mut buf).map_err(|_| ())?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}

fn read_dict(cur: &mut std::io::Cursor<&[u8]>) -> Result<HashMap<String, String>, ()> {
    use byteorder::{LittleEndian, ReadBytesExt};
    let n = cur.read_i32::<LittleEndian>().map_err(|_| ())?.max(0) as usize;
    let mut out: HashMap<String, String> = HashMap::with_capacity(n);
    for _ in 0..n {
        let k = read_string(cur)?;
        let v = read_string(cur)?;
        out.insert(k, v);
    }
    Ok(out)
}

fn trn_local(trn: &Trn) -> Xform {
    let mut x = Xform::identity();
    let f = trn.frames.first().unwrap_or(&trn.attrs);
    if let Some(t) = f.get("_t") {
        let mut it = t.split_whitespace();
        let tx = it.next().and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
        let ty = it.next().and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
        let tz = it.next().and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
        x.t = [tx, ty, tz];
    }
    if let Some(r) = f.get("_r").or_else(|| trn.attrs.get("_r")) {
        if let Ok(rv) = r.parse::<u8>() { x.m = decode_rot(rv); }
    }
    x
}

fn decode_rot(r: u8) -> [[i32; 3]; 3] {
    let r1 = (r & 3) as usize;
    let r2 = ((r >> 2) & 3) as usize;
    let s1 = if ((r >> 4) & 1) == 0 { 1 } else { -1 };
    let s2 = if ((r >> 5) & 1) == 0 { 1 } else { -1 };
    let s3 = if ((r >> 6) & 1) == 0 { 1 } else { -1 };
    let mut rem = vec![0usize, 1, 2];
    rem.retain(|i| *i != r1 && *i != r2);
    let r3 = rem.get(0).copied().unwrap_or(2);
    let mut m = [[0i32; 3]; 3];
    m[0][r1] = s1;
    m[1][r2] = s2;
    m[2][r3] = s3;
    m
}

fn visit_node(id: i32, parent: Xform, trns: &HashMap<i32, Trn>, grps: &HashMap<i32, Grp>, shps: &HashMap<i32, Shp>, out: &mut Vec<(usize, Xform)>) {
    if let Some(t) = trns.get(&id) {
        let local = trn_local(t);
        let world = parent.mul(&local);
        visit_node(t.child, world, trns, grps, shps, out);
        return;
    }
    if let Some(g) = grps.get(&id) {
        for c in g.children.iter().copied() { visit_node(c, parent, trns, grps, shps, out); }
        return;
    }
    if let Some(s) = shps.get(&id) {
        for mid in s.models.iter().copied() {
            if mid >= 0 { out.push((mid as usize, parent)); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vox_import_smoke() {
        let p = Path::new("example/voxel-model-master/vox/procedure/ff1.vox");
        let bytes = std::fs::read(p).expect("missing sample .vox");
        let v = parse_vox(&bytes).expect("parse_vox failed");
        assert!(!v.voxels.is_empty(), "parsed voxels are empty");
    }
}

