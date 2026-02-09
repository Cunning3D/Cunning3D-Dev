use flatbuffers::{FlatBufferBuilder, TableFinishedWIPOffset, TableUnfinishedWIPOffset, WIPOffset};
use crate::algorithms::algorithms_runtime::unity_spline::editor::harness::{KnotRef, KnotSnapshot, M4, Q4, SplineContainerSnapshot, SplineSnapshot, V3};
use crate::algorithms::algorithms_runtime::unity_spline::{BezierKnot, KnotLinkCollection, MetaData, Spline, SplineContainer, SplineKnotIndex, TangentMode, CATMULL_ROM_TENSION};
use crate::coord::basis::{self, BasisId};

// Minimal self-contained FlatBuffers encoder for schemas/cunning_spline_snapshot.fbs (no flatc step).

mod fbs {
    use super::*;

    #[derive(Clone, Copy)]
    pub struct Vec3(pub V3);
    #[derive(Clone, Copy)]
    pub struct Quat(pub Q4);
    #[derive(Clone, Copy)]
    pub struct Mat4(pub M4);
    #[derive(Clone, Copy)]
    pub struct KnotRefS(pub KnotRef);

    impl flatbuffers::Push for Vec3 {
        type Output = Self;
        #[inline] unsafe fn push(&self, dst: &mut [u8], _written_len: usize) { dst[..12].copy_from_slice(bytemuck::cast_slice(&self.0.0)); }
    }
    impl flatbuffers::Push for Quat {
        type Output = Self;
        #[inline] unsafe fn push(&self, dst: &mut [u8], _written_len: usize) { dst[..16].copy_from_slice(bytemuck::cast_slice(&self.0.0)); }
    }
    impl flatbuffers::Push for Mat4 {
        type Output = Self;
        #[inline] unsafe fn push(&self, dst: &mut [u8], _written_len: usize) {
            let cols = self.0.0;
            let mut off = 0;
            for c in 0..4 { dst[off..off + 16].copy_from_slice(bytemuck::cast_slice(&cols[c])); off += 16; }
        }
    }
    impl flatbuffers::Push for KnotRefS {
        type Output = Self;
        #[inline] unsafe fn push(&self, dst: &mut [u8], _written_len: usize) {
            let s = self.0.0[0]; let k = self.0.0[1];
            dst[..4].copy_from_slice(&s.to_le_bytes());
            dst[4..8].copy_from_slice(&k.to_le_bytes());
        }
    }

    const KNOT_POS: flatbuffers::VOffsetT = 4;
    const KNOT_TIN: flatbuffers::VOffsetT = 6;
    const KNOT_TOUT: flatbuffers::VOffsetT = 8;
    const KNOT_ROT: flatbuffers::VOffsetT = 10;
    const KNOT_MODE: flatbuffers::VOffsetT = 12;
    const KNOT_TENSION: flatbuffers::VOffsetT = 14;

    const SPLINE_CLOSED: flatbuffers::VOffsetT = 4;
    const SPLINE_KNOTS: flatbuffers::VOffsetT = 6;

    const LG_KNOTS: flatbuffers::VOffsetT = 4;

    const SCHEMA_VER: flatbuffers::VOffsetT = 4;
    const SC_L2W: flatbuffers::VOffsetT = 6;
    const SC_SPLINES: flatbuffers::VOffsetT = 8;
    const SC_LINKS: flatbuffers::VOffsetT = 10;

    pub fn create_knot<'bldr>(b: &mut FlatBufferBuilder<'bldr>, k: &KnotSnapshot) -> WIPOffset<TableFinishedWIPOffset> {
        let st: WIPOffset<TableUnfinishedWIPOffset> = b.start_table();
        b.push_slot_always(KNOT_POS, Vec3(k.position));
        b.push_slot_always(KNOT_TIN, Vec3(k.tangent_in));
        b.push_slot_always(KNOT_TOUT, Vec3(k.tangent_out));
        b.push_slot_always(KNOT_ROT, Quat(k.rotation));
        b.push_slot_always(KNOT_MODE, k.mode.0 as u8);
        b.push_slot_always(KNOT_TENSION, k.tension);
        b.end_table(st)
    }

    pub fn create_spline<'bldr>(b: &mut FlatBufferBuilder<'bldr>, s: &SplineSnapshot) -> WIPOffset<TableFinishedWIPOffset> {
        let knots: Vec<_> = s.knots.iter().map(|k| create_knot(b, k)).collect();
        let knots_vec = b.create_vector(&knots);
        let st: WIPOffset<TableUnfinishedWIPOffset> = b.start_table();
        b.push_slot_always(SPLINE_CLOSED, s.closed);
        b.push_slot_always(SPLINE_KNOTS, knots_vec);
        b.end_table(st)
    }

    pub fn create_link_group<'bldr>(b: &mut FlatBufferBuilder<'bldr>, g: &[KnotRef]) -> WIPOffset<TableFinishedWIPOffset> {
        let structs: Vec<KnotRefS> = g.iter().map(|k| KnotRefS(*k)).collect();
        let v = b.create_vector(&structs);
        let st: WIPOffset<TableUnfinishedWIPOffset> = b.start_table();
        b.push_slot_always(LG_KNOTS, v);
        b.end_table(st)
    }

    pub fn create_snapshot<'bldr>(b: &mut FlatBufferBuilder<'bldr>, snap: &SplineContainerSnapshot) -> WIPOffset<TableFinishedWIPOffset> {
        let splines: Vec<_> = snap.splines.iter().map(|sp| create_spline(b, sp)).collect();
        let splines_vec = b.create_vector(&splines);
        let groups: Vec<Vec<KnotRef>> = snap.links.iter().map(|g| g.iter().map(|k| KnotRef(k.0)).collect()).collect();
        let lgoffs: Vec<_> = groups.iter().map(|g| create_link_group(b, g)).collect();
        let links_vec = b.create_vector(&lgoffs);
        let st: WIPOffset<TableUnfinishedWIPOffset> = b.start_table();
        b.push_slot_always(SCHEMA_VER, 1u32);
        b.push_slot_always(SC_L2W, Mat4(snap.local_to_world));
        b.push_slot_always(SC_SPLINES, splines_vec);
        b.push_slot_always(SC_LINKS, links_vec);
        b.end_table(st)
    }
}

pub fn encode_snapshot_fbs(snapshot: &SplineContainerSnapshot) -> Vec<u8> {
    let mut b = FlatBufferBuilder::new();
    let root = fbs::create_snapshot(&mut b, snapshot);
    b.finish(root, None);
    b.finished_data().to_vec()
}

#[inline] fn u32le(b: &[u8], at: usize) -> Option<u32> { Some(u32::from_le_bytes(b.get(at..at+4)?.try_into().ok()?)) }
#[inline] fn i32le(b: &[u8], at: usize) -> Option<i32> { Some(i32::from_le_bytes(b.get(at..at+4)?.try_into().ok()?)) }
#[inline] fn u16le(b: &[u8], at: usize) -> Option<u16> { Some(u16::from_le_bytes(b.get(at..at+2)?.try_into().ok()?)) }
#[inline] fn f32le(b: &[u8], at: usize) -> Option<f32> { Some(f32::from_le_bytes(b.get(at..at+4)?.try_into().ok()?)) }
#[inline] fn vtbl_field(b: &[u8], table: usize, vo: u16) -> Option<usize> {
    let vt = (table as isize - i32le(b, table)? as isize) as usize;
    let off = u16le(b, vt + vo as usize)? as usize;
    if off == 0 { None } else { Some(table + off) }
}
#[inline] fn vec_pos(b: &[u8], field: usize) -> Option<usize> { Some(field + u32le(b, field)? as usize) }
#[inline] fn vec_len(b: &[u8], v: usize) -> Option<usize> { Some(u32le(b, v)? as usize) }
#[inline] fn vec_u32_off(b: &[u8], v: usize, i: usize) -> Option<usize> { let e = v + 4 + i * 4; Some(e + u32le(b, e)? as usize) }
#[inline] fn read_v3(b: &[u8], at: usize) -> Option<V3> { let a: [u8; 12] = b.get(at..at+12)?.try_into().ok()?; Some(V3(bytemuck::cast(a))) }
#[inline] fn read_q4(b: &[u8], at: usize) -> Option<Q4> { let a: [u8; 16] = b.get(at..at+16)?.try_into().ok()?; Some(Q4(bytemuck::cast(a))) }
#[inline] fn read_m4(b: &[u8], at: usize) -> Option<M4> {
    let mut m = [[0f32; 4]; 4]; let mut off = at;
    for c in 0..4 { let a: [u8; 16] = b.get(off..off+16)?.try_into().ok()?; m[c] = bytemuck::cast(a); off += 16; }
    Some(M4(m))
}

pub fn decode_snapshot_fbs(bytes: &[u8]) -> Option<SplineContainerSnapshot> {
    const SCHEMA_VER: u16 = 4; const SC_L2W: u16 = 6; const SC_SPLINES: u16 = 8; const SC_LINKS: u16 = 10;
    const SPLINE_CLOSED: u16 = 4; const SPLINE_KNOTS: u16 = 6;
    const KNOT_POS: u16 = 4; const KNOT_TIN: u16 = 6; const KNOT_TOUT: u16 = 8; const KNOT_ROT: u16 = 10; const KNOT_MODE: u16 = 12; const KNOT_TENSION: u16 = 14;
    const LG_KNOTS: u16 = 4;
    let root = u32le(bytes, 0)? as usize;
    let ver = vtbl_field(bytes, root, SCHEMA_VER).and_then(|p| u32le(bytes, p)).unwrap_or(0);
    if ver != 1 { return None; }
    let l2w = read_m4(bytes, vtbl_field(bytes, root, SC_L2W)?)?;
    let sv = vec_pos(bytes, vtbl_field(bytes, root, SC_SPLINES)?)?;
    let sn = vec_len(bytes, sv)?;
    let mut splines: Vec<SplineSnapshot> = Vec::with_capacity(sn);
    for si in 0..sn {
        let st = vec_u32_off(bytes, sv, si)?;
        let closed = vtbl_field(bytes, st, SPLINE_CLOSED).and_then(|p| bytes.get(p).copied()).unwrap_or(0) != 0;
        let kv = vec_pos(bytes, vtbl_field(bytes, st, SPLINE_KNOTS)?)?;
        let kn = vec_len(bytes, kv)?;
        let mut knots: Vec<KnotSnapshot> = Vec::with_capacity(kn);
        for ki in 0..kn {
            let kt = vec_u32_off(bytes, kv, ki)?;
            let pos = read_v3(bytes, vtbl_field(bytes, kt, KNOT_POS)?)?;
            let tin = read_v3(bytes, vtbl_field(bytes, kt, KNOT_TIN)?)?;
            let tout = read_v3(bytes, vtbl_field(bytes, kt, KNOT_TOUT)?)?;
            let rot = read_q4(bytes, vtbl_field(bytes, kt, KNOT_ROT)?)?;
            let mode = vtbl_field(bytes, kt, KNOT_MODE).and_then(|p| bytes.get(p).copied()).unwrap_or(4);
            let tension = vtbl_field(bytes, kt, KNOT_TENSION).and_then(|p| f32le(bytes, p)).unwrap_or(CATMULL_ROM_TENSION);
            knots.push(KnotSnapshot { position: pos, tangent_in: tin, tangent_out: tout, rotation: rot, mode: crate::algorithms::algorithms_runtime::unity_spline::editor::harness::ModeJson(match mode { 0 => TangentMode::AutoSmooth, 1 => TangentMode::Linear, 2 => TangentMode::Mirrored, 3 => TangentMode::Continuous, _ => TangentMode::Broken }), tension });
        }
        splines.push(SplineSnapshot { closed, knots });
    }
    let lv = vec_pos(bytes, vtbl_field(bytes, root, SC_LINKS)?)?;
    let ln = vec_len(bytes, lv)?;
    let mut links: Vec<Vec<KnotRef>> = Vec::with_capacity(ln);
    for gi in 0..ln {
        let gt = vec_u32_off(bytes, lv, gi)?;
        let gv = vec_pos(bytes, vtbl_field(bytes, gt, LG_KNOTS)?)?;
        let gn = vec_len(bytes, gv)?;
        let mut g: Vec<KnotRef> = Vec::with_capacity(gn);
        for i in 0..gn {
            let at = gv + 4 + i * 8;
            let s = i32le(bytes, at)?;
            let k = i32le(bytes, at + 4)?;
            g.push(KnotRef([s, k]));
        }
        links.push(g);
    }
    Some(SplineContainerSnapshot { splines, links, local_to_world: l2w })
}

pub fn decode_snapshot_fbs_to_container(bytes: &[u8], source_basis: u32) -> Option<SplineContainer> {
    let snap = decode_snapshot_fbs(bytes)?;
    let map = if source_basis == 1 { basis::map(BasisId::Unity, BasisId::InternalBevy) } else { None };
    let mut c = SplineContainer::default();
    let mut l2w = bevy::math::Mat4::from_cols_array_2d(&snap.local_to_world.0);
    if let Some(m) = map { l2w = m.map_m4(l2w); }
    c.local_to_world = l2w;
    for s in snap.splines.iter() {
        let mut sp = Spline::default();
        sp.closed = s.closed;
        for k in s.knots.iter() {
            let mut pos = bevy::math::Vec3::from_array(k.position.0);
            let mut tin = bevy::math::Vec3::from_array(k.tangent_in.0);
            let mut tout = bevy::math::Vec3::from_array(k.tangent_out.0);
            let mut rot = bevy::math::Quat::from_xyzw(k.rotation.0[0], k.rotation.0[1], k.rotation.0[2], k.rotation.0[3]);
            if let Some(m) = map { pos = m.map_v3(pos); tin = m.map_v3(tin); tout = m.map_v3(tout); rot = m.map_q(rot); }
            sp.knots.push(BezierKnot { position: pos, tangent_in: tin, tangent_out: tout, rotation: rot });
            sp.meta.push(MetaData::new(k.mode.0, k.tension));
        }
        c.splines.push(sp);
    }
    c.links = KnotLinkCollection::default();
    for g in snap.links.iter() {
        if g.len() < 2 { continue; }
        let a = g[0].0; let a = SplineKnotIndex::new(a[0], a[1]);
        for k in g.iter().skip(1) { let x = k.0; c.link_knots(a, SplineKnotIndex::new(x[0], x[1])); }
    }
    Some(c)
}

#[cfg(target_arch = "wasm32")]
pub fn encode_snapshot_fbs_zstd(snapshot: &SplineContainerSnapshot, _level: i32) -> Vec<u8> {
    encode_snapshot_fbs(snapshot)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn encode_snapshot_fbs_zstd(snapshot: &SplineContainerSnapshot, _level: i32) -> Vec<u8> {
    encode_snapshot_fbs(snapshot)
}

