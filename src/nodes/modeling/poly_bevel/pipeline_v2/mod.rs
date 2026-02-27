use super::structures::{BevelGraph, BevelParams, BevelWorkspace, MiterType, Profile};
use crate::libs::geometry::attrs;
use crate::libs::geometry::ids::{HalfEdgeId, PointId, PrimId, VertexId};
use crate::libs::geometry::mesh::{Attribute, GeoPrimitive, GeoVertex, Geometry, PolygonPrim};
use crate::libs::geometry::topology::Topology;
use bevy::prelude::*;
use rayon::prelude::*;
use std::collections::HashMap;
use super::spokes::spoke_fan;

pub mod adjust_offsets;
pub mod boundary;
pub mod build_poly;
pub mod custom_profile;
pub mod cutoff;
pub mod face_rebuild;
pub mod loop_slide;
pub mod math;
pub mod offset_limit;
pub mod pipe;
pub mod profile;
#[path = "Profiles/mod.rs"]
pub mod profiles;
pub mod spacing;
pub mod terminal_edge;
pub mod tri_corner;
pub mod vertex_only;
pub mod vmesh;
pub mod weld;

/// Internal polygon representation used by the bevel pipeline: point indices into `out_p`.
pub type Poly = Vec<usize>;

use math::*;
use pipe::{pipe_snap, pipe_test};
use profile::build_profile_samples;
#[allow(unused_imports)]
use profiles::miter::{adjust_miter_coords, miter_test};
use profiles::square_out::try_square_out_adj_vmesh;
use spacing::build_profile_spacing;
#[allow(unused_imports)]
use tri_corner::{tri_corner_test, tri_corner_vmesh};
use vmesh::{cubic_subdiv, emit_vmesh_faces, interp_vmesh, VMeshGrid};

// Thread-local workspace cache (avoid repeated allocations across calls)
thread_local! {
    static WORKSPACE: std::cell::RefCell<BevelWorkspace> = std::cell::RefCell::new(BevelWorkspace::new());
}

pub struct BevelPipeline<'a> {
    geo: &'a Geometry,
    topo: &'a Topology,
    positions: &'a [Vec3],
    graph: BevelGraph,
    params: BevelParams,
    new_geo: Geometry,
}

impl<'a> BevelPipeline<'a> {
    pub fn new(
        geo: &'a Geometry,
        topo: &'a Topology,
        graph: BevelGraph,
        invert_profile: bool,
    ) -> Self {
        Self::new_with_corner_scale(geo, topo, graph, invert_profile, 1.0, PRO_CIRCLE_R)
    }

    pub fn new_with_corner_scale(
        geo: &'a Geometry,
        topo: &'a Topology,
        graph: BevelGraph,
        invert_profile: bool,
        corner_scale: f32,
        profile_r: f32,
    ) -> Self {
        let mut params = BevelParams::default();
        params.invert_profile = invert_profile;
        params.corner_scale = corner_scale;
        params.pro_super_r = profile_r;
        Self::new_with_params(geo, topo, graph, params)
    }

    pub fn new_with_params(
        geo: &'a Geometry,
        topo: &'a Topology,
        graph: BevelGraph,
        params: BevelParams,
    ) -> Self {
        let positions = geo.get_point_position_attribute().expect("Missing @P");
        Self {
            geo,
            topo,
            positions,
            graph,
            params,
            new_geo: Geometry::new(),
        }
    }

    fn get_pos(&self, id: PointId) -> Vec3 {
        self.geo
            .points()
            .get_dense_index(id.into())
            .and_then(|di| self.positions.get(di).copied())
            .unwrap_or(Vec3::ZERO)
    }

    pub fn execute(mut self) -> Geometry {
        self.build_output();
        self.new_geo
    }

    fn build_output(&mut self) {
        // v2: high-performance bevel pipeline; uses an object pool to avoid repeated allocations
        let mut dist = 0.0f32;
        let mut divisions = 1usize;
        for e in &self.graph.edges {
            if e.is_bev {
                dist = dist.max(e.offset_l.max(e.offset_r));
                divisions = divisions.max(e.seg.max(1));
            }
        }

        // Calculate dynamic epsilon based on geometry bounds
        let (min, max) = self
            .geo
            .compute_bounds()
            .unwrap_or((Vec3::ZERO, Vec3::ZERO));
        let bbox_size = (max - min).length();
        let eps = BevelEpsilon::new(bbox_size);

        if dist <= eps.eps {
            self.new_geo = self.geo.clone();
            return;
        }

        // [OFFSET_LIMIT] Apply offset clamping if enabled (Blender bevel_limit_offset)
        if self.params.clamp_overlap && !self.params.debug_disable_offset_limit {
            let edge_lengths: Vec<f32> = self
                .graph
                .edges
                .iter()
                .map(|e| {
                    let bv = self.graph.verts.get(e.origin_bev_vert);
                    let pair = self.graph.edges.get(e.pair_index);
                    match (bv, pair) {
                        (Some(v1), Some(p)) => {
                            let v2 = self.graph.verts.get(p.origin_bev_vert);
                            match v2 {
                                Some(v2) => {
                                    (self.get_pos(v1.p_id) - self.get_pos(v2.p_id)).length()
                                }
                                None => 1.0,
                            }
                        }
                        _ => 1.0,
                    }
                })
                .collect();
            let positions = self.positions;
            let positions_fn = |pi: usize| positions.get(pi).copied().unwrap_or(Vec3::ZERO);
            let edge_len_fn = |ei: usize| edge_lengths.get(ei).copied().unwrap_or(1.0);
            let (limited, was_limited) = offset_limit::limit_offset(
                &mut self.graph,
                &self.params,
                &positions_fn,
                &edge_len_fn,
            );
            if was_limited {
                dist = limited;
            }
        }

        let is_vertex_bevel = vertex_only::is_vertex_only(&self.params);
        let n_points = self.geo.points().len();
        let n_prims = self.geo.primitives().len();
        let n_halfedges = self.topo.half_edges.len();

        // Reuse memory via thread-local workspace
        let (
            mut bev_edge,
            mut any_bev_point,
            mut out_p,
            mut out_polys,
            mut corner_pt,
            mut face_n_dense,
            mut dense_to_pid,
            mut dense_to_he,
        ) = WORKSPACE.with(|ws| {
            let mut ws = ws.borrow_mut();
            ws.prepare(n_points, n_prims, n_halfedges, self.graph.edges.len());
            (
                std::mem::take(&mut ws.bev_edge),
                std::mem::take(&mut ws.any_bev_point),
                std::mem::take(&mut ws.out_points),
                std::mem::take(&mut ws.out_polys),
                std::mem::take(&mut ws.corner_pt),
                std::mem::take(&mut ws.face_normals_dense),
                std::mem::take(&mut ws.dense_to_pid),
                std::mem::take(&mut ws.dense_to_he),
            )
        });
        let mut out_poly_src_face: Vec<PrimId> = Vec::with_capacity(out_polys.capacity());
        let mut out_poly_corner_n: Vec<Vec<Vec3>> = Vec::with_capacity(out_polys.capacity());
        let face_n_of = |prim_id: PrimId, face_n_dense: &[Vec3], geo: &Geometry| -> Vec3 {
            geo.primitives()
                .get_dense_index(prim_id.into())
                .and_then(|di| face_n_dense.get(di).copied())
                .unwrap_or(Vec3::Y)
        };
        let face_n_of_he =
            |he: HalfEdgeId, face_n_dense: &[Vec3], topo: &Topology, geo: &Geometry| -> Vec3 {
                topo.half_edges
                    .get(he.into())
                    .map(|h| face_n_of(h.primitive_index, face_n_dense, geo))
                    .unwrap_or(Vec3::Y)
            };

        // Dense index vec for HalfEdgeId -> graph edge index (replaces HashMap for O(1) access)
        let mut he_to_eidx: Vec<u32> = vec![u32::MAX; n_halfedges];
        for (ei, e) in self.graph.edges.iter().enumerate() {
            if let Some(di) = self.topo.half_edges.get_dense_index(e.he_id.into()) {
                if di < he_to_eidx.len() {
                    he_to_eidx[di] = ei as u32;
                }
            }
            if !e.is_bev {
                continue;
            }
            let he = e.he_id;
            let pair = self.topo.pair(he);
            if !pair.is_valid() {
                continue;
            }
            let canon = if pair.index() < he.index() { pair } else { he };
            if let Some(cdi) = self.topo.half_edges.get_dense_index(canon.into()) {
                if cdi < bev_edge.len() {
                    bev_edge[cdi] = true;
                }
            }
            let Some(hen) = self.topo.half_edges.get(he.into()) else {
                continue;
            };
            let u = hen.origin_point;
            let v = self.topo.dest_point(he);
            if let Some(udi) = self.geo.points().get_dense_index(u.into()) {
                if udi < any_bev_point.len() {
                    any_bev_point[udi] = true;
                }
            }
            if let Some(vdi) = self.geo.points().get_dense_index(v.into()) {
                if vdi < any_bev_point.len() {
                    any_bev_point[vdi] = true;
                }
            }
        }

        // Helper: get graph edge index from HalfEdgeId via dense index
        let get_eidx = |he: HalfEdgeId, he_to_eidx: &[u32]| -> Option<usize> {
            self.topo
                .half_edges
                .get_dense_index(he.into())
                .and_then(|di| {
                    he_to_eidx.get(di).and_then(|&v| {
                        if v == u32::MAX {
                            None
                        } else {
                            Some(v as usize)
                        }
                    })
                })
        };

        let mut orig_pt: Vec<Option<usize>> = vec![None; n_points];
        // Parallel face normal computation
        let prims_with_start: Vec<_> = self
            .topo
            .primitive_to_halfedge
            .iter()
            .filter(|(_, &start)| start.is_valid())
            .map(|(&prim_id, &start)| (prim_id, start))
            .collect();
        let normals: Vec<_> = prims_with_start
            .par_iter()
            .map(|&(prim_id, _)| (prim_id, self.compute_face_normal(prim_id)))
            .collect();
        for (prim_id, n) in normals {
            if let Some(di) = self.geo.primitives().get_dense_index(prim_id.into()) {
                if di < face_n_dense.len() {
                    face_n_dense[di] = n;
                }
            }
        }
        if self.params.debug_invert_face_normals {
            for n in &mut face_n_dense {
                *n = -*n;
            }
        }

        // Initialize the dense mapping table (workspace is preallocated)
        for (pid_idx, _) in self.geo.points().iter_enumerated() {
            let pid = PointId::from(pid_idx);
            if let Some(di) = self.geo.points().get_dense_index(pid.into()) {
                if di < dense_to_pid.len() {
                    dense_to_pid[di] = pid;
                }
            }
        }
        for (he_idx, _) in self.topo.half_edges.iter_enumerated() {
            let hid = HalfEdgeId::from(he_idx);
            if let Some(di) = self.topo.half_edges.get_dense_index(he_idx) {
                if di < dense_to_he.len() {
                    dense_to_he[di] = hid;
                }
            }
        }
        let add_point = |pos: Vec3, out_p: &mut Vec<Vec3>| -> usize {
            let idx = out_p.len();
            out_p.push(pos);
            idx
        };

        // 1) corner_pt + BoundVert ring (Blender build_boundary / adjust_offsets / build_boundary(construct=false))
        let debug_spoke_order = self.params.debug_spoke_order;
        let mut debug_spokes = |spokes: &mut Vec<HalfEdgeId>| {
            if spokes.len() <= 1 {
                return;
            }
            match debug_spoke_order {
                1 => spokes.reverse(),
                2 => spokes.rotate_left(1),
                3 => spokes.rotate_right(1),
                _ => {}
            }
        };
        for pass in 0..2 {
            let mut pid_to_bv_idx: HashMap<PointId, usize> =
                HashMap::with_capacity(self.graph.verts.len());
            for (i, v) in self.graph.verts.iter().enumerate() {
                pid_to_bv_idx.insert(v.p_id, i);
            }
            self.graph.bound_verts.clear();
            for e in self.graph.edges.iter_mut() {
                e.left_v = None;
                e.right_v = None;
            }
            for (pi, pid) in dense_to_pid.iter().enumerate() {
                if !pid.is_valid() {
                    continue;
                }
                if orig_pt.get(pi).and_then(|o| *o).is_none() {
                    orig_pt[pi] = Some(add_point(self.get_pos(*pid), &mut out_p));
                }
            }
            for (he_idx, he) in self.topo.half_edges.iter_enumerated() {
                let Some(di) = self.topo.half_edges.get_dense_index(he_idx) else {
                    continue;
                };
                let Some(pdi) = self.geo.points().get_dense_index(he.origin_point.into()) else {
                    continue;
                };
                if let Some(op) = orig_pt.get(pdi).and_then(|o| *o) {
                    if di < corner_pt.len() {
                        corner_pt[di] = op as u32;
                    }
                }
            }

            for (pi, involved) in any_bev_point.iter().enumerate() {
                if !*involved {
                    continue;
                }
                let p = dense_to_pid.get(pi).copied().unwrap_or(PointId::INVALID);
                if !p.is_valid() {
                    continue;
                }
                let mut spokes: Vec<HalfEdgeId> = spoke_fan(self.topo, p);
                debug_spokes(&mut spokes);
                if spokes.len() < 2 {
                    continue;
                }
                let v_pos = self.get_pos(p);
                let mut spoke_is_bev: Vec<bool> = Vec::with_capacity(spokes.len());
                let mut edge_dirs: Vec<Vec3> = Vec::with_capacity(spokes.len());
                let mut edge_ends: Vec<Vec3> = Vec::with_capacity(spokes.len());
                let mut spoke_off_l: Vec<f32> = Vec::with_capacity(spokes.len());
                let mut spoke_off_r: Vec<f32> = Vec::with_capacity(spokes.len());
                let mut face_nos: Vec<Vec3> = Vec::with_capacity(spokes.len());
                let mut pair_face_nos: Vec<Vec3> = Vec::with_capacity(spokes.len());
                for &he in &spokes {
                    let ei = get_eidx(he, &he_to_eidx);
                    if let Some(i) = ei {
                        let e = &self.graph.edges[i];
                        spoke_is_bev.push(e.is_bev);
                        spoke_off_l.push(e.offset_l);
                        spoke_off_r.push(e.offset_r);
                    } else {
                        spoke_is_bev.push(false);
                        spoke_off_l.push(0.0);
                        spoke_off_r.push(0.0);
                    }
                    let end = self.get_pos(self.topo.dest_point(he));
                    edge_ends.push(end);
                    edge_dirs.push((end - v_pos).normalize_or_zero());
                    face_nos.push(face_n_of_he(he, &face_n_dense, self.topo, self.geo));
                    let pair = self.topo.pair(he);
                    pair_face_nos.push(if pair.is_valid() {
                        face_n_of_he(pair, &face_n_dense, self.topo, self.geo)
                    } else {
                        Vec3::ZERO
                    });
                }
                if self.params.debug_swap_offsets_lr {
                    std::mem::swap(&mut spoke_off_l, &mut spoke_off_r);
                }
                if self.params.debug_swap_face_pair_normals {
                    std::mem::swap(&mut face_nos, &mut pair_face_nos);
                }
                if self.params.debug_invert_pair_face_normals {
                    for n in &mut pair_face_nos {
                        *n = -*n;
                    }
                }
                if self.params.debug_invert_edge_ends {
                    for e in &mut edge_ends {
                        *e = v_pos + (v_pos - *e);
                    }
                }
                if self.params.debug_invert_edge_dirs {
                    for d in &mut edge_dirs {
                        *d = -*d;
                    }
                }
                let selcount = spoke_is_bev.iter().filter(|&&b| b).count();
                if selcount == 0 {
                    continue;
                }
                let (bnd_verts, edges_info, mesh_kind) = if selcount == 1 {
                    let bev_idx = spoke_is_bev.iter().position(|&b| b).unwrap_or(0);
                    let offset = get_eidx(spokes[bev_idx], &he_to_eidx)
                        .map(|ei| self.graph.edges[ei].offset_l)
                        .unwrap_or(dist);
                    let t = terminal_edge::build_terminal_edge_boundary(
                        spokes.len(),
                        bev_idx,
                        &edge_dirs,
                        &face_nos,
                        v_pos,
                        offset,
                        &self.params,
                    );
                    (t.bnd_verts, None, t.mesh_kind)
                } else {
                    let lite = boundary::BevelParamsLite {
                        offset: dist,
                        seg: divisions,
                        loop_slide: self.params.loop_slide,
                        miter_outer: match self.params.miter_outer {
                            MiterType::Patch => boundary::MiterKind::Patch,
                            MiterType::Arc => boundary::MiterKind::Arc,
                            _ => boundary::MiterKind::Sharp,
                        },
                        miter_inner: match self.params.miter_inner {
                            MiterType::Arc => boundary::MiterKind::Arc,
                            _ => boundary::MiterKind::Sharp,
                        },
                        spread: self.params.spread,
                    };
                    let mut b = boundary::build_boundary_lite(
                        &spokes,
                        &spoke_is_bev,
                        &edge_dirs,
                        &edge_ends,
                        &spoke_off_l,
                        &spoke_off_r,
                        &face_nos,
                        &pair_face_nos,
                        v_pos,
                        &lite,
                    );
                    if self.params.debug_swap_left_right {
                        for e in &mut b.edges {
                            std::mem::swap(&mut e.left_bv, &mut e.right_bv);
                        }
                    }
                    let mk = if b.bnd_verts.len() <= 2 {
                        super::structures::MeshKind::None
                    } else if divisions <= 1 {
                        super::structures::MeshKind::Poly
                    } else {
                        super::structures::MeshKind::Adj
                    };
                    (b.bnd_verts, Some(b.edges), mk)
                };
                if bnd_verts.is_empty() {
                    continue;
                }

                // Write BoundVert ring into graph (used by adjust_offsets/rebuild parity later).
                if let Some(&bv_idx) = pid_to_bv_idx.get(&p) {
                    let base = self.graph.bound_verts.len();
                    for (i, bv) in bnd_verts.iter().enumerate() {
                        let e_idx = bv
                            .efirst
                            .and_then(|si| spokes.get(si))
                            .and_then(|&he| get_eidx(he, &he_to_eidx));
                        let efirst = bv
                            .efirst
                            .and_then(|si| spokes.get(si))
                            .and_then(|&he| get_eidx(he, &he_to_eidx));
                        let elast = bv
                            .elast
                            .and_then(|si| spokes.get(si))
                            .and_then(|&he| get_eidx(he, &he_to_eidx));
                        let eon = bv
                            .eon
                            .and_then(|si| spokes.get(si))
                            .and_then(|&he| get_eidx(he, &he_to_eidx));
                        self.graph.bound_verts.push(super::structures::BoundVert {
                            pos: bv.pos,
                            next: base + bv.next.min(bnd_verts.len().saturating_sub(1)),
                            prev: base + bv.prev.min(bnd_verts.len().saturating_sub(1)),
                            index: i,
                            is_arc_start: bv.is_arc_start,
                            is_patch_start: bv.is_patch_start,
                            efirst,
                            elast,
                            eon,
                            sinratio: bv.sinratio,
                            profile: super::structures::Profile::default(),
                            e_idx,
                        });
                    }
                    if let Some(v) = self.graph.verts.get_mut(bv_idx) {
                        v.vmesh = Some(super::structures::VMesh {
                            bound_start: base,
                            count: bnd_verts.len(),
                            seg: divisions,
                            kind: mesh_kind,
                            mesh_verts: Vec::new(),
                        });
                    }
                    if let Some(edges) = edges_info.as_ref() {
                        for (si, info) in edges.iter().enumerate() {
                            let Some(he) = spokes.get(si).copied() else {
                                continue;
                            };
                            let Some(ei) = get_eidx(he, &he_to_eidx) else {
                                continue;
                            };
                            if let Some(li) = info.left_bv {
                                self.graph.edges[ei].left_v = Some(base + li);
                            }
                            if let Some(ri) = info.right_bv {
                                self.graph.edges[ei].right_v = Some(base + ri);
                            }
                        }
                    } else {
                        // Terminal edge: derive left/right bv mapping from BoundVertLite ownership.
                        let mut per_spoke: Vec<Vec<usize>> = vec![Vec::new(); spokes.len()];
                        for (li, bv) in bnd_verts.iter().enumerate() {
                            if let Some(si) = bv.efirst {
                                if si < per_spoke.len() {
                                    per_spoke[si].push(base + li);
                                }
                            }
                        }
                        for (si, ids) in per_spoke.into_iter().enumerate() {
                            let Some(he) = spokes.get(si).copied() else {
                                continue;
                            };
                            let Some(ei) = get_eidx(he, &he_to_eidx) else {
                                continue;
                            };
                            if ids.is_empty() {
                                continue;
                            }
                            self.graph.edges[ei].left_v = Some(ids[0]);
                            self.graph.edges[ei].right_v = Some(*ids.get(1).unwrap_or(&ids[0]));
                        }
                    }
                }

                let mut sectors: Vec<((usize, usize), u32)> = Vec::with_capacity(bnd_verts.len());
                let mut bnd_pts: Vec<u32> = Vec::with_capacity(bnd_verts.len());
                for bv in &bnd_verts {
                    let pidx = add_point(bv.pos, &mut out_p) as u32;
                    bnd_pts.push(pidx);
                    let (Some(a), Some(b)) = (bv.efirst, bv.elast) else {
                        continue;
                    };
                    sectors.push(((a, b), pidx));
                }
                if sectors.is_empty() {
                    continue;
                }
                for &he_out in &spokes {
                    let he_prev = self.topo.prev(he_out);
                    let he_in = self.topo.pair(he_prev);
                    if !he_in.is_valid() {
                        continue;
                    }
                    let i_out = spokes
                        .iter()
                        .position(|&x| x == he_out)
                        .unwrap_or(usize::MAX);
                    let i_in = spokes
                        .iter()
                        .position(|&x| x == he_in)
                        .unwrap_or(usize::MAX);
                    if i_out == usize::MAX || i_in == usize::MAX {
                        continue;
                    }
                    let exact_bv = |bv_idx: usize| -> bool {
                        bnd_verts
                            .get(bv_idx)
                            .map(|bv| {
                                bv.efirst == Some(i_in) && bv.elast == Some(i_out)
                            })
                            .unwrap_or(false)
                    };
                    // Prefer edge-local left/right ownership first (most stable for per-face corner).
                    let mut pt: Option<u32> = edges_info
                        .as_ref()
                        .and_then(|edges| edges.get(i_out))
                        .and_then(|info| {
                            if let Some(li) = info.left_bv {
                                if exact_bv(li) {
                                    return bnd_pts.get(li).copied();
                                }
                            }
                            if let Some(ri) = info.right_bv {
                                if exact_bv(ri) {
                                    return bnd_pts.get(ri).copied();
                                }
                            }
                            info.left_bv
                                .and_then(|li| bnd_pts.get(li).copied())
                                .or_else(|| info.right_bv.and_then(|ri| bnd_pts.get(ri).copied()))
                        });
                    if pt.is_none() {
                        // Fallback: sector matching by (incoming -> outgoing), then range.
                        pt = sectors
                            .iter()
                            .find(|((a, b), _)| *a == i_in && *b == i_out)
                            .or_else(|| {
                                sectors.iter().find(|((a, b), _)| {
                                    if *a == *b {
                                        return i_out == *a;
                                    }
                                    if *a < *b {
                                        i_out > *a && i_out <= *b
                                    } else {
                                        i_out > *a || i_out <= *b
                                    }
                                })
                            })
                            .map(|(_, p)| *p);
                    }
                    if let (Some(pt), Some(di)) =
                        (pt, self.topo.half_edges.get_dense_index(he_out.into()))
                    {
                        if di < corner_pt.len() {
                            corner_pt[di] = pt;
                        }
                    }
                }
            }

            if pass == 0
                && self.params.loop_slide
                && !self.params.debug_disable_adjust_offsets
                && adjust_offsets::adjust_offsets(&mut self.graph, 6)
            {
                continue;
            }
            break;
        }

        // Helper: get corner point from dense index
        let get_corner_pt = |he: HalfEdgeId, corner_pt: &Vec<u32>| -> Option<usize> {
            self.topo
                .half_edges
                .get_dense_index(he.into())
                .and_then(|di| {
                    corner_pt.get(di).and_then(|&v| {
                        if v == u32::MAX {
                            None
                        } else {
                            Some(v as usize)
                        }
                    })
                })
        };

        // 2) [REBUILD_PREP] We no longer emit inset faces here. Rebuild of original faces is done as the final step
        // (Blender bev_rebuild_polygon semantics): after edge strips + corner vmesh are emitted.

        // 3) profiles
        let pro_r = self.params.pro_super_r;
        let pro_spacing = build_profile_spacing(divisions, pro_r);
        let mut spoke_profiles: HashMap<HalfEdgeId, Profile> = HashMap::new();
        for (pi, involved) in any_bev_point.iter().enumerate() {
            if !*involved {
                continue;
            }
            let p = dense_to_pid.get(pi).copied().unwrap_or(PointId::INVALID);
            if !p.is_valid() {
                continue;
            }
            let mut spokes: Vec<HalfEdgeId> = spoke_fan(self.topo, p);
            debug_spokes(&mut spokes);
            if spokes.len() < 2 {
                continue;
            }
            let selcount = spokes
                .iter()
                .filter(|he| {
                    get_eidx(**he, &he_to_eidx)
                        .map(|ei| self.graph.edges[ei].is_bev)
                        .unwrap_or(false)
                })
                .count();

            // [TERMINAL_EDGE] Handle single beveled edge case (Blender build_boundary_terminal_edge)
            if selcount == 1 {
                let u_pos = self.get_pos(p);
                let edge_dirs: Vec<Vec3> = spokes
                    .iter()
                    .map(|&he| (self.get_pos(self.topo.dest_point(he)) - u_pos).normalize_or_zero())
                    .collect();
                let face_nos: Vec<Vec3> = spokes
                    .iter()
                    .map(|&he| face_n_of_he(he, &face_n_dense, self.topo, self.geo))
                    .collect();
                let bev_idx = spokes
                    .iter()
                    .position(|he| {
                        get_eidx(*he, &he_to_eidx)
                            .map(|ei| self.graph.edges[ei].is_bev)
                            .unwrap_or(false)
                    })
                    .unwrap_or(0);
                let offset = get_eidx(spokes[bev_idx], &he_to_eidx)
                    .map(|ei| self.graph.edges[ei].offset_l)
                    .unwrap_or(dist);
                let _term_result = terminal_edge::build_terminal_edge_boundary(
                    spokes.len(),
                    bev_idx,
                    &edge_dirs,
                    &face_nos,
                    u_pos,
                    offset,
                    &self.params,
                );
                // Terminal edge handling generates its own boundary; for now we continue with standard profile generation
            }

            for (i, &he) in spokes.iter().enumerate() {
                let u_pos = self.get_pos(p);
                let v_pos = self.get_pos(self.topo.dest_point(he));
                let start = get_corner_pt(he, &corner_pt)
                    .and_then(|idx| out_p.get(idx).copied())
                    .unwrap_or(u_pos);
                let he_lr = {
                    let pair = self.topo.pair(he);
                    if pair.is_valid() {
                        self.topo.next(pair)
                    } else {
                        HalfEdgeId::INVALID
                    }
                };
                let end = if he_lr.is_valid() {
                    get_corner_pt(he_lr, &corner_pt)
                        .and_then(|idx| out_p.get(idx).copied())
                        .unwrap_or(start)
                } else {
                    start
                };
                let eidx = get_eidx(he, &he_to_eidx);
                let (prev_dir, next_dir) = if let Some(ei) = eidx {
                    let e = &self.graph.edges[ei];
                    let pd = self
                        .get_pos(self.topo.dest_point(self.graph.edges[e.prev_index].he_id))
                        - u_pos;
                    let nd = self
                        .get_pos(self.topo.dest_point(self.graph.edges[e.next_index].he_id))
                        - u_pos;
                    (Some(pd.normalize_or_zero()), Some(nd.normalize_or_zero()))
                } else {
                    (None, None)
                };
                let cp = if matches!(
                    self.params.profile_shape,
                    super::structures::ProfileShape::Custom
                ) {
                    self.params.custom_profile.as_ref()
                } else {
                    None
                };
                let (mut prof, _ps) = build_profile_samples(
                    divisions,
                    self.params.invert_profile,
                    pro_r,
                    cp,
                    start,
                    end,
                    u_pos,
                    v_pos,
                    is_vertex_bevel,
                    selcount,
                    prev_dir,
                    next_dir,
                );
                prof.face_start = self
                    .topo
                    .half_edges
                    .get(he.into())
                    .map(|h| h.primitive_index)
                    .unwrap_or(PrimId::INVALID);
                prof.face_end = if he_lr.is_valid() {
                    self.topo
                        .half_edges
                        .get(he_lr.into())
                        .map(|h| h.primitive_index)
                        .unwrap_or(prof.face_start)
                } else {
                    prof.face_start
                };
                let is_bev = eidx.map(|ei| self.graph.edges[ei].is_bev).unwrap_or(false);
                if !is_bev {
                    for k in 0..=divisions {
                        let t = k as f32 / divisions as f32;
                        if let Some(p) = prof.prof_co.get_mut(k) {
                            *p = start.lerp(end, t);
                        }
                    }
                    if prof.prof_co_2.len() > 1 {
                        let m = prof.prof_co_2.len() - 1;
                        for k in 0..=m {
                            prof.prof_co_2[k] = start.lerp(end, k as f32 / m as f32);
                        }
                    }
                }
                // ids: boundary reuse
                let idx_start = get_corner_pt(he, &corner_pt)
                    .unwrap_or_else(|| add_point(prof.start, &mut out_p));
                let idx_end = if he_lr.is_valid() {
                    get_corner_pt(he_lr, &corner_pt)
                        .unwrap_or_else(|| add_point(prof.end, &mut out_p))
                } else {
                    idx_start
                };
                prof.ids = std::iter::once(idx_start)
                    .chain((1..divisions).map(|k| add_point(prof.prof_co[k], &mut out_p)))
                    .chain(std::iter::once(idx_end))
                    .collect();

                // [MITER] Miter test available: miter_test(selcount, angle_kinds, miter_outer, miter_inner)
                // Miter geometry generation in vmesh phase when needed

                spoke_profiles.insert(he, prof);
                let _ = i;
            }
        }

        // 3.5) build_square_in_vmesh: weld profile ids around tri-corner (Blender bmesh_bevel.cc 5127+)
        // [CIRCLE_GUARD] This block is ONLY for SquareIn (pro_r == PRO_SQUARE_IN_R). Circle path skips this entirely.
        let mut square_in_skip = vec![false; any_bev_point.len()];
        let is_circle = (pro_r - PRO_CIRCLE_R).abs() < 1e-3;
        let is_square_in = (pro_r - PRO_SQUARE_IN_R).abs() < 1e-6;
        let is_square_out = (pro_r - PRO_SQUARE_R).abs() < 1e-3;
        let _ = is_circle; // [CIRCLE_GUARD] Circle is the default path; SquareIn/SquareOut are opt-in branches.
        if is_square_in && divisions > 1 && !self.params.debug_disable_square_in_vmesh {
            let ns = divisions;
            let ns2 = ns / 2;
            let odd = (ns & 1) == 1;
            for (pi, involved) in any_bev_point.iter().enumerate() {
                if !*involved {
                    continue;
                }
                let p = dense_to_pid.get(pi).copied().unwrap_or(PointId::INVALID);
                if !p.is_valid() {
                    continue;
                }
                let mut spokes_all: Vec<HalfEdgeId> = spoke_fan(self.topo, p);
                debug_spokes(&mut spokes_all);
                let mut spokes: Vec<HalfEdgeId> = Vec::new();
                for &he in &spokes_all {
                    if get_eidx(he, &he_to_eidx)
                        .map(|ei| self.graph.edges[ei].is_bev)
                        .unwrap_or(false)
                    {
                        spokes.push(he);
                    }
                }
                if spokes.len() != 3 {
                    continue;
                }
                let selcount = 3usize;
                let offsets: Vec<f32> = spokes_all
                    .iter()
                    .map(|he| {
                        get_eidx(*he, &he_to_eidx)
                            .map(|ei| self.graph.edges[ei].offset_l)
                            .unwrap_or(0.0)
                    })
                    .collect();
                let is_bev: Vec<bool> = spokes_all
                    .iter()
                    .map(|he| {
                        get_eidx(*he, &he_to_eidx)
                            .map(|ei| self.graph.edges[ei].is_bev)
                            .unwrap_or(false)
                    })
                    .collect();
                if tri_corner_test(
                    &spokes_all,
                    selcount,
                    pro_r,
                    &offsets,
                    &is_bev,
                    &face_n_dense,
                    &|he| self.topo.pair(he),
                    &|he| {
                        self.topo.half_edges.get(he.into()).and_then(|h| {
                            self.geo
                                .primitives()
                                .get_dense_index(h.primitive_index.into())
                        })
                    },
                    &|he| Some(self.get_pos(self.topo.dest_point(he)) - self.get_pos(p)),
                ) != 1
                {
                    continue;
                }
                // Order the 3 profiles like Blender boundstart->next->next, but only using this corner's `profs`
                // (avoid borrowing `spoke_profiles` twice; keep Circle path untouched).
                let mut profs: Vec<&Profile> = Vec::new();
                for he in &spokes {
                    if let Some(pr) = spoke_profiles.get(he) {
                        profs.push(pr);
                    } else {
                        profs.clear();
                        break;
                    }
                }
                if profs.len() != 3 {
                    continue;
                }
                let faces: Vec<PrimId> = spokes
                    .iter()
                    .filter_map(|he| {
                        self.topo
                            .half_edges
                            .get((*he).into())
                            .map(|h| h.primitive_index)
                    })
                    .collect();
                let mut ordered_idx: Vec<usize> = Vec::with_capacity(3);
                for i in 0..3 {
                    let f0 = faces[i];
                    let f1 = faces[(i + 1) % 3];
                    let mut found: Option<usize> = None;
                    for (j, pr) in profs.iter().enumerate() {
                        if (pr.face_start == f0 && pr.face_end == f1)
                            || (pr.face_start == f1 && pr.face_end == f0)
                        {
                            found = Some(j);
                            break;
                        }
                    }
                    if let Some(j) = found {
                        ordered_idx.push(j);
                    } else {
                        ordered_idx.clear();
                        break;
                    }
                }
                if ordered_idx.len() != 3 {
                    continue;
                }
                let ordered_hes: [HalfEdgeId; 3] = [
                    spokes[ordered_idx[0]],
                    spokes[ordered_idx[1]],
                    spokes[ordered_idx[2]],
                ];
                let ordered_starts: [Vec3; 3] = [
                    profs[ordered_idx[0]].start,
                    profs[ordered_idx[1]].start,
                    profs[ordered_idx[2]].start,
                ];
                let mut ids: Vec<Vec<usize>> = Vec::with_capacity(3);
                for &he in &ordered_hes {
                    let Some(pr) = spoke_profiles.get(&he) else {
                        ids.clear();
                        break;
                    };
                    if pr.ids.len() != ns + 1 {
                        ids.clear();
                        break;
                    }
                    ids.push(pr.ids.clone());
                }
                if ids.len() != 3 {
                    continue;
                }

                for i in 0..3 {
                    for k in 1..ns {
                        if i > 0 && k <= ns2 {
                            ids[i][k] = ids[i - 1][ns - k];
                        } else if i == 2 && k > ns2 {
                            ids[i][k] = ids[0][ns - k];
                        }
                    }
                }
                // Copy boundary coordinates from tri-corner vmesh (Blender copies vm1->vm and then reuses verts).
                if let Some(vm_tri) = tri_corner_vmesh(
                    ordered_starts[0],
                    ordered_starts[1],
                    ordered_starts[2],
                    self.get_pos(p),
                    ns,
                    pro_r,
                ) {
                    for i in 0..3 {
                        for k in 0..=ns {
                            let id = ids[i][k];
                            if let Some(pp) = out_p.get_mut(id) {
                                *pp = vm_tri.get(i, 0, k);
                            }
                        }
                    }
                }

                // Write back ids to the 3 actual bevel spokes for this corner (no global scan).
                for i in 0..3 {
                    if let Some(pr) = spoke_profiles.get_mut(&ordered_hes[i]) {
                        pr.ids = ids[i].clone();
                    }
                }
                square_in_skip[pi] = true;
                if odd {
                    let mut tri = vec![ids[0][ns2], ids[1][ns2], ids[2][ns2]];
                    let mut c = Vec3::ZERO;
                    for &id in &tri {
                        c += out_p[id];
                    }
                    c /= 3.0;
                    let outward = (self.get_pos(p) - c).normalize_or_zero();
                    let n = (out_p[tri[1]] - out_p[tri[0]])
                        .cross(out_p[tri[2]] - out_p[tri[0]])
                        .normalize_or_zero();
                    if n.dot(outward) < 0.0 {
                        tri.reverse();
                    }
                    out_polys.push(tri);
                    out_poly_src_face.push(PrimId::INVALID);
                    out_poly_corner_n.push(Vec::new());
                }
            }
        }

        // 4) edge strips
        for canon_idx in 0..bev_edge.len() {
            if !bev_edge[canon_idx] {
                continue;
            }
            let he = dense_to_he
                .get(canon_idx)
                .copied()
                .unwrap_or(HalfEdgeId::INVALID);
            if !he.is_valid() {
                continue;
            }
            let pair = self.topo.pair(he);
            if !pair.is_valid() {
                continue;
            }
            let (Some(prof_u), Some(prof_v)) = (spoke_profiles.get(&he), spoke_profiles.get(&pair))
            else {
                continue;
            };
            if prof_u.ids.len() != divisions + 1 || prof_v.ids.len() != divisions + 1 {
                continue;
            }
            let face_a = self
                .topo
                .half_edges
                .get(he.into())
                .map(|h| h.primitive_index)
                .unwrap_or(PrimId::INVALID);
            let orient = |p: &Profile| -> Vec<usize> {
                if p.face_start == face_a {
                    p.ids.clone()
                } else if p.face_end == face_a {
                    let mut v = p.ids.clone();
                    v.reverse();
                    v
                } else {
                    p.ids.clone()
                }
            };
            let p_u = orient(prof_u);
            let p_v = orient(prof_v);
            let a_n = self.compute_face_normal(face_a);
            let face_b = self
                .topo
                .half_edges
                .get(pair.into())
                .map(|h| h.primitive_index)
                .unwrap_or(face_a);
            let slerp_n = |a: Vec3, b: Vec3, t: f32| -> Vec3 {
                let a = a.normalize_or_zero();
                let b = b.normalize_or_zero();
                let dot = a.dot(b).clamp(-1.0, 1.0);
                let om = dot.acos();
                if om.abs() < 1e-6 {
                    return a;
                }
                let so = om.sin();
                ((a * ((1.0 - t) * om).sin() + b * (t * om).sin()) / so).normalize_or_zero()
            };
            let (na, nb) = (
                face_n_of(face_a, &face_n_dense, self.geo),
                face_n_of(face_b, &face_n_dense, self.geo),
            );
            for i in 0..divisions {
                let (u0, u1, v0, v1) = (p_u[i], p_u[i + 1], p_v[i], p_v[i + 1]);
                let qa = out_p[v0] - out_p[u0];
                let qb = out_p[u1] - out_p[u0];
                let quad_fwd =
                    self.params.debug_disable_strip_orient || qa.cross(qb).dot(a_n) >= 0.0;
                let quad = if quad_fwd {
                    vec![u0, v0, v1, u1]
                } else {
                    vec![u0, u1, v1, v0]
                };
                out_polys.push(quad);
                out_poly_src_face.push(PrimId::INVALID);
                if self.params.harden_normals {
                    let t0 = i as f32 / divisions as f32;
                    let t1 = (i + 1) as f32 / divisions as f32;
                    let n0 = slerp_n(na, nb, t0);
                    let n1 = slerp_n(na, nb, t1);
                    out_poly_corner_n.push(if quad_fwd {
                        vec![n0, n0, n1, n1]
                    } else {
                        vec![n0, n1, n1, n0]
                    });
                } else {
                    out_poly_corner_n.push(Vec::new());
                }
            }
        }

        // 5) corners (vmesh)
        for (pi, involved) in any_bev_point.iter().enumerate() {
            if !*involved {
                continue;
            }
            if square_in_skip.get(pi).copied().unwrap_or(false) {
                continue;
            }
            let p = dense_to_pid.get(pi).copied().unwrap_or(PointId::INVALID);
            if !p.is_valid() {
                continue;
            }
            let mut spokes_all: Vec<HalfEdgeId> = spoke_fan(self.topo, p);
            debug_spokes(&mut spokes_all);
            if spokes_all.len() < 3 || divisions < 2 {
                continue;
            }
            let corner_pos = self.get_pos(p);

            // [VERTEX_ONLY] Handle vertex-only bevel mode
            if is_vertex_bevel {
                // Collect all edges around this vertex (not just beveled ones)
                let edge_dirs: Vec<Vec3> = spokes_all
                    .iter()
                    .map(|&he| {
                        (self.get_pos(self.topo.dest_point(he)) - corner_pos).normalize_or_zero()
                    })
                    .collect();
                let face_normals: Vec<Vec3> = spokes_all
                    .iter()
                    .map(|&he| face_n_of_he(he, &face_n_dense, self.topo, self.geo))
                    .collect();

                let (vertex_pts, vertex_polys) = vertex_only::build_vertex_only_vmesh(
                    corner_pos,
                    &edge_dirs,
                    &face_normals,
                    dist,
                    divisions,
                    pro_r,
                );

                // Add vertex-only geometry to output
                let vertex_base = out_p.len();
                for pt in vertex_pts {
                    out_p.push(pt);
                }
                for poly in vertex_polys {
                    let mapped: Vec<usize> = poly.iter().map(|&i| vertex_base + i).collect();
                    out_polys.push(mapped);
                    out_poly_src_face.push(PrimId::INVALID);
                    out_poly_corner_n.push(Vec::new());
                }
                continue; // Skip standard edge bevel processing
            }

            let mut spokes: Vec<HalfEdgeId> = Vec::new();
            for &he in &spokes_all {
                if get_eidx(he, &he_to_eidx)
                    .map(|ei| self.graph.edges[ei].is_bev)
                    .unwrap_or(false)
                {
                    spokes.push(he);
                }
            }

            if spokes.len() < 2 {
                continue;
            }

            let mut profs = Vec::new();
            for he in &spokes {
                if let Some(pr) = spoke_profiles.get(he) {
                    profs.push(pr);
                } else {
                    profs.clear();
                    break;
                }
            }
            if profs.is_empty() {
                continue;
            }
            let selcount = spokes.len();
            let (bnd, arc_for, spoke_dirs_all, face_nos_all, pair_face_nos_all) = {
                let mut spoke_dirs_all: Vec<Vec3> = Vec::with_capacity(spokes_all.len());
                let mut spoke_ends_all: Vec<Vec3> = Vec::with_capacity(spokes_all.len());
                let mut spoke_is_bev_all: Vec<bool> = Vec::with_capacity(spokes_all.len());
                let mut spoke_off_l_all: Vec<f32> = Vec::with_capacity(spokes_all.len());
                let mut spoke_off_r_all: Vec<f32> = Vec::with_capacity(spokes_all.len());
                let mut face_nos_all: Vec<Vec3> = Vec::with_capacity(spokes_all.len());
                let mut pair_face_nos_all: Vec<Vec3> = Vec::with_capacity(spokes_all.len());
                for &he in &spokes_all {
                    let end = self.get_pos(self.topo.dest_point(he));
                    spoke_ends_all.push(end);
                    spoke_dirs_all.push((end - corner_pos).normalize_or_zero());
                    let ei = get_eidx(he, &he_to_eidx);
                    if let Some(i) = ei {
                        let e = &self.graph.edges[i];
                        spoke_is_bev_all.push(e.is_bev);
                        spoke_off_l_all.push(e.offset_l);
                        spoke_off_r_all.push(e.offset_r);
                    } else {
                        spoke_is_bev_all.push(false);
                        spoke_off_l_all.push(0.0);
                        spoke_off_r_all.push(0.0);
                    }
                    face_nos_all.push(face_n_of_he(he, &face_n_dense, self.topo, self.geo));
                    let pair = self.topo.pair(he);
                    pair_face_nos_all.push(if pair.is_valid() {
                        face_n_of_he(pair, &face_n_dense, self.topo, self.geo)
                    } else {
                        Vec3::ZERO
                    });
                }
                if self.params.debug_swap_offsets_lr {
                    std::mem::swap(&mut spoke_off_l_all, &mut spoke_off_r_all);
                }
                if self.params.debug_swap_face_pair_normals {
                    std::mem::swap(&mut face_nos_all, &mut pair_face_nos_all);
                }
                if self.params.debug_invert_pair_face_normals {
                    for n in &mut pair_face_nos_all {
                        *n = -*n;
                    }
                }
                if self.params.debug_invert_edge_ends {
                    for e in &mut spoke_ends_all {
                        *e = corner_pos + (corner_pos - *e);
                    }
                }
                if self.params.debug_invert_edge_dirs {
                    for d in &mut spoke_dirs_all {
                        *d = -*d;
                    }
                }
                let lite = boundary::BevelParamsLite {
                    offset: dist,
                    seg: divisions,
                    loop_slide: self.params.loop_slide,
                    miter_outer: match self.params.miter_outer {
                        MiterType::Patch => boundary::MiterKind::Patch,
                        MiterType::Arc => boundary::MiterKind::Arc,
                        _ => boundary::MiterKind::Sharp,
                    },
                    miter_inner: match self.params.miter_inner {
                        MiterType::Arc => boundary::MiterKind::Arc,
                        _ => boundary::MiterKind::Sharp,
                    },
                    spread: self.params.spread,
                };
                let mut bnd = boundary::build_boundary_lite(
                    &spokes_all,
                    &spoke_is_bev_all,
                    &spoke_dirs_all,
                    &spoke_ends_all,
                    &spoke_off_l_all,
                    &spoke_off_r_all,
                    &face_nos_all,
                    &pair_face_nos_all,
                    corner_pos,
                    &lite,
                );
                if self.params.debug_swap_left_right {
                    for e in &mut bnd.edges {
                        std::mem::swap(&mut e.left_bv, &mut e.right_bv);
                    }
                }
                let mut arc_for: Vec<Option<(usize, bool)>> = vec![None; bnd.count];
                for (s, info) in bnd.edges.iter().enumerate() {
                    if !info.is_bev {
                        continue;
                    }
                    let (Some(l), Some(r)) = (info.left_bv, info.right_bv) else {
                        continue;
                    };
                    if l < bnd.bnd_verts.len()
                        && r < bnd.bnd_verts.len()
                        && bnd.bnd_verts[l].next == r
                    {
                        arc_for[l] = Some((s, true));
                    }
                    if r < bnd.bnd_verts.len()
                        && l < bnd.bnd_verts.len()
                        && bnd.bnd_verts[r].next == l
                    {
                        arc_for[r] = Some((s, false));
                    }
                }
                if self.params.debug_invert_arc_for {
                    for a in &mut arc_for {
                        if let Some((_, fwd)) = a.as_mut() {
                            *fwd = !*fwd;
                        }
                    }
                }
                (
                    bnd,
                    arc_for,
                    spoke_dirs_all,
                    face_nos_all,
                    pair_face_nos_all,
                )
            };
            let n_bndv = bnd.count;
            if n_bndv < 2 {
                continue;
            }

            // [WELD] Exactly 2 beveled edges at a 3-valence corner (Blender weld: selcount==2 && vm->count==2).
            // Important: we must EMIT corner connection faces here; skipping would leave a visible gap/flat patch.
            if weld::is_weld_case(selcount, n_bndv) && n_bndv == 2 && divisions > 0 {
                let he1 = spokes[0];
                let he2 = spokes[1];
                if divisions > 1 {
                    if let (Some(p1), Some(p2)) = (
                        spoke_profiles.get(&he1).cloned(),
                        spoke_profiles.get(&he2).cloned(),
                    ) {
                        if p1.ids.len() > 2 && p2.ids.len() > 2 {
                            for k in 1..divisions {
                                let k2 = divisions - k;
                                if k < p1.ids.len() && k2 < p2.ids.len() {
                                    let (i1, i2) = (p1.ids[k], p2.ids[k2]);
                                    out_p[i1] = (out_p[i1] + out_p[i2]) * 0.5;
                                }
                            }
                            if let Some(p2m) = spoke_profiles.get_mut(&he2) {
                                for k in 1..divisions {
                                    let k2 = divisions - k;
                                    if k < p1.ids.len() && k2 < p2m.ids.len() {
                                        p2m.ids[k2] = p1.ids[k];
                                    }
                                }
                            }
                        }
                    }
                }
                let (Some(p1), Some(p2)) = (spoke_profiles.get(&he1), spoke_profiles.get(&he2))
                else {
                    continue;
                };
                if p1.ids.len() < divisions + 1 || p2.ids.len() < divisions + 1 {
                    continue;
                }
                // Fill the weld corner by directly stitching the two profiles (Blender weld corner behavior).
                // Use the common face between the two profiles as reference for orientation.
                let common_face = {
                    let a = [p1.face_start, p1.face_end];
                    let b = [p2.face_start, p2.face_end];
                    a.into_iter()
                        .find(|fa| fa.is_valid() && b.contains(fa))
                        .unwrap_or(p1.face_start)
                };
                let ref_n = face_n_of(common_face, &face_n_dense, self.geo).normalize_or_zero();
                let orient_ids = |pr: &Profile| -> Vec<usize> {
                    if pr.face_start == common_face {
                        pr.ids.clone()
                    } else if pr.face_end == common_face {
                        let mut v = pr.ids.clone();
                        v.reverse();
                        v
                    } else {
                        pr.ids.clone()
                    }
                };
                let a = orient_ids(p1);
                let b = orient_ids(p2);
                for k in 0..divisions {
                    let (a0, a1, b0, b1) = (a[k], a[k + 1], b[k], b[k + 1]);
                    let qa = out_p[b0] - out_p[a0];
                    let qb = out_p[a1] - out_p[a0];
                    let mut quad = vec![a0, a1, b1, b0];
                    if qa.cross(qb).dot(ref_n) < 0.0 {
                        quad.reverse();
                    }
                    out_polys.push(quad);
                    out_poly_src_face.push(PrimId::INVALID);
                    out_poly_corner_n.push(Vec::new());
                }
                continue;
            }
            let bnd_prof = |i: usize| {
                arc_for.get(i).and_then(|o| *o).and_then(|(s, fwd)| {
                    spokes_all
                        .get(s)
                        .and_then(|&he| spoke_profiles.get(&he).map(|pr| (pr, fwd)))
                })
            };
            let bnd_profs: Vec<&Profile> = (0..n_bndv)
                .map(|i| bnd_prof(i).map(|(p, _)| p).unwrap_or(profs[0]))
                .collect();
            let prof_at = |i: usize, k: usize, nseg: usize| -> Vec3 {
                let Some((pr, fwd)) = bnd_prof(i) else {
                    return bnd.bnd_verts.get(i).map(|bv| bv.pos).unwrap_or(corner_pos);
                };
                if nseg == divisions {
                    let kk = if fwd {
                        k.min(divisions)
                    } else {
                        divisions.saturating_sub(k.min(divisions))
                    };
                    pr.prof_co.get(kk).copied().unwrap_or(pr.start)
                } else {
                    let seg_2 = pro_spacing.seg_2.max(4);
                    if nseg > 0 && (seg_2 % nseg) == 0 && !pr.prof_co_2.is_empty() {
                        let step = (seg_2 / nseg).max(1);
                        let kk = (k * step).min(seg_2);
                        let kk = if fwd { kk } else { seg_2.saturating_sub(kk) };
                        pr.prof_co_2.get(kk).copied().unwrap_or(pr.start)
                    } else {
                        let (s0, s1) = if fwd {
                            (pr.start, pr.end)
                        } else {
                            (pr.end, pr.start)
                        };
                        s0.lerp(s1, k as f32 / nseg as f32)
                    }
                }
            };
            let mut vm0 = VMeshGrid::new(n_bndv, 2);
            for i in 0..n_bndv {
                for k in 0..=2 {
                    vm0.set(i, 0, k, prof_at(i, k, 2));
                }
            }
            let mut bnd_center = Vec3::ZERO;
            for i in 0..n_bndv {
                bnd_center += bnd.bnd_verts.get(i).map(|bv| bv.pos).unwrap_or(Vec3::ZERO);
            }
            bnd_center /= n_bndv as f32;
            vm0.set(
                0,
                1,
                1,
                bnd_center + (corner_pos - bnd_center) * pro_spacing.fullness.clamp(0.0, 1.0),
            );
            vm0.copy_equiv();
            let mut vm1 = vm0;
            while vm1.ns < divisions {
                vm1 = cubic_subdiv(vm1, &prof_at);
            }
            if vm1.ns != divisions {
                vm1 = interp_vmesh(vm1, &prof_at, divisions);
            }

            // Blender square_out_adj_vmesh: PRO_SQUARE_R + selcount>=3 + even segments (driven by real boundary + face normals).
            // [CIRCLE_GUARD] This block is ONLY for SquareOut. Circle path skips this entirely.
            if is_square_out
                && selcount >= 3
                && (divisions & 1) == 0
                && !self.params.debug_disable_square_out_adj_vmesh
            {
                let profiles: Vec<Vec3> = bnd.bnd_verts.iter().map(|bv| bv.pos).collect();
                let middles: Vec<Vec3> = bnd
                    .bnd_verts
                    .iter()
                    .map(|bv| {
                        if bv.is_arc_start {
                            bv.arc_middle
                        } else {
                            bv.pos
                        }
                    })
                    .collect();
                if let Some(vm) = try_square_out_adj_vmesh(
                    &bnd,
                    &profiles,
                    &middles,
                    &spoke_dirs_all,
                    &face_nos_all,
                    &pair_face_nos_all,
                    corner_pos,
                    divisions,
                ) {
                    vm1 = vm;
                }
            }
            // [WELD] Special handling for 2 beveled edges meeting (Blender 6092-6176)
            let selcount = spokes.len();
            let is_weld = weld::is_weld_case(selcount, n_bndv);

            // Blender pipe_test + pipe_adj_vmesh snapping (v2 module).
            let mut did_pipe = false;
            if let Some((ipipe, pipe_dir)) = pipe_test(
                n_bndv,
                selcount,
                &spokes,
                corner_pos,
                &face_n_dense,
                &|idx| {
                    get_eidx(spokes[idx], &he_to_eidx)
                        .map(|ei| self.graph.edges[ei].is_bev)
                        .unwrap_or(false)
                },
                &|he| {
                    self.topo.half_edges.get(he.into()).and_then(|h| {
                        self.geo
                            .primitives()
                            .get_dense_index(h.primitive_index.into())
                    })
                },
                &|he| Some(self.get_pos(self.topo.dest_point(he))),
                &eps,
            ) {
                // Blender pipe_adj_vmesh (4828+): even square profiles may snap to midline for some vertices.
                // [CIRCLE_GUARD] This Square-specific snapping is ONLY for SquareOut. Circle uses generic pipe_snap.
                if is_square_out && (divisions & 1) == 0 {
                    let ns = divisions;
                    let half_ns = ns / 2;
                    let ipipe1 = ipipe;
                    let ipipe2 = (ipipe + 2) % n_bndv;
                    let pro = bnd_profs[ipipe1 % n_bndv];
                    for i in 0..n_bndv {
                        for j in 1..=half_ns {
                            for k in 0..=half_ns {
                                if !vm1.is_canon(i, j, k) {
                                    continue;
                                }
                                let midline = k == half_ns
                                    && ((i == 0 && j == half_ns) || i == ipipe1 || i == ipipe2);
                                let co = vm1.get(i, j, k);
                                vm1.set(
                                    i,
                                    j,
                                    k,
                                    pipe::snap_to_pipe_profile(pro, pipe_dir, midline, co, &eps),
                                );
                            }
                        }
                    }
                    vm1.copy_equiv();
                } else {
                    pipe_snap(
                        &mut vm1, divisions, n_bndv, ipipe, pipe_dir, &bnd_profs, &eps,
                    );
                }
                did_pipe = true;
            }
            // [MITER] Apply outer miter adjustments if enabled (Blender adjust_miter_coords)
            // This adds extra geometry at sharp outer corners to prevent self-intersection
            let miter_outer = self.params.miter_outer;
            let miter_inner = self.params.miter_inner;
            if !matches!(miter_outer, MiterType::Sharp) && n_bndv >= 3 {
                // Calculate angle kinds for each edge around the vertex
                let mut angle_kinds: Vec<boundary::AngleKind> = Vec::new();
                for j in 0..n_bndv {
                    let prev_dir = profs[(j + n_bndv - 1) % n_bndv].start - corner_pos;
                    let curr_dir = profs[j].start - corner_pos;
                    let angle = prev_dir
                        .normalize_or_zero()
                        .dot(curr_dir.normalize_or_zero())
                        .acos();
                    if angle < 0.5 {
                        angle_kinds.push(boundary::AngleKind::Smaller);
                    } else if angle > 2.6 {
                        angle_kinds.push(boundary::AngleKind::Larger);
                    } else {
                        angle_kinds.push(boundary::AngleKind::Straight);
                    }
                }
                let (needs_outer, outer_idx, _needs_inner) =
                    profiles::miter::miter_test(selcount, &angle_kinds, miter_outer, miter_inner);
                if needs_outer {
                    if let Some(idx) = outer_idx {
                        // Build boundary verts for miter
                        let spoke_dirs: Vec<Vec3> = profs
                            .iter()
                            .map(|p| (p.end - p.start).normalize_or_zero())
                            .collect();
                        let bndv_positions: Vec<boundary::BoundVertLite> = profs
                            .iter()
                            .enumerate()
                            .map(|(i, p)| boundary::BoundVertLite {
                                pos: p.start,
                                index: i,
                                efirst: Some(i),
                                elast: Some(i),
                                ..Default::default()
                            })
                            .collect();

                        // Get miter adjustments
                        let adjustments = profiles::miter::adjust_miter_coords(
                            &bndv_positions,
                            idx,
                            corner_pos,
                            &spoke_dirs,
                            miter_outer,
                            dist,
                            divisions,
                        );

                        // Apply adjustments to profile starts
                        for (adj_idx, new_pos) in adjustments {
                            if adj_idx < profs.len() {
                                // Create extra miter polygon vertices
                                let miter_pt_idx = add_point(new_pos, &mut out_p);
                                // Connect miter vertex to adjacent profile starts
                                let prev_idx = if adj_idx == 0 {
                                    n_bndv - 1
                                } else {
                                    adj_idx - 1
                                };
                                let next_idx = (adj_idx + 1) % n_bndv;
                                if let (Some(prev_start), Some(next_start)) =
                                    (profs[prev_idx].ids.first(), profs[next_idx].ids.first())
                                {
                                    // Create miter triangle
                                    out_polys.push(vec![*prev_start, miter_pt_idx, *next_start]);
                                    out_poly_src_face.push(PrimId::INVALID);
                                    out_poly_corner_n.push(Vec::new());
                                }
                            }
                        }
                    }
                }
            }

            // Blender tri_corner_adj_vmesh: strict tri_corner_test + cube-corner vmesh mapping (boundary-driven).
            // Circle and SquareOut can use this path; only SquareIn is excluded.
            if !did_pipe && n_bndv == 3 && !is_square_in {
                let offsets: Vec<f32> = spokes_all
                    .iter()
                    .map(|he| {
                        get_eidx(*he, &he_to_eidx)
                            .map(|ei| self.graph.edges[ei].offset_l)
                            .unwrap_or(0.0)
                    })
                    .collect();
                let is_bev: Vec<bool> = spokes_all
                    .iter()
                    .map(|he| {
                        get_eidx(*he, &he_to_eidx)
                            .map(|ei| self.graph.edges[ei].is_bev)
                            .unwrap_or(false)
                    })
                    .collect();
                if tri_corner_test(
                    &spokes_all,
                    selcount,
                    pro_r,
                    &offsets,
                    &is_bev,
                    &face_n_dense,
                    &|he| self.topo.pair(he),
                    &|he| {
                        self.topo.half_edges.get(he.into()).and_then(|h| {
                            self.geo
                                .primitives()
                                .get_dense_index(h.primitive_index.into())
                        })
                    },
                    &|he| Some(self.get_pos(self.topo.dest_point(he)) - corner_pos),
                ) == 1
                {
                    let b0 = bnd.bnd_verts[0].pos;
                    let b1 = bnd.bnd_verts[1].pos;
                    let b2 = bnd.bnd_verts[2].pos;
                    if let Some(vm_tri) = tri_corner_vmesh(b0, b1, b2, corner_pos, divisions, pro_r)
                    {
                        vm1 = vm_tri;
                    }
                }
            }
            // [WELD] handled earlier (before vmesh build) to avoid skipping corner fill or emitting duplicates.

            // [VMESH_METHOD] Check if cutoff method should be used instead of grid fill
            let use_cutoff = matches!(
                self.params.vmesh_method,
                super::structures::VMeshMethod::Cutoff
            );

            if use_cutoff {
                // Build cutoff faces instead of standard vmesh grid
                let boundary_id = |ci: usize, ck: usize| -> Option<usize> {
                    arc_for.get(ci).and_then(|o| *o).and_then(|(s, fwd)| {
                        let he = *spokes_all.get(s)?;
                        let pr = spoke_profiles.get(&he)?;
                        let k = if fwd {
                            ck
                        } else {
                            divisions.saturating_sub(ck)
                        };
                        pr.ids.get(k).copied()
                    })
                };
                let boundary_pts: Vec<Vec3> = bnd.bnd_verts.iter().map(|bv| bv.pos).collect();
                let mut profile_pts: Vec<Vec<Vec3>> = Vec::with_capacity(bnd.count);
                for ci in 0..bnd.count {
                    let mut row: Vec<Vec3> = Vec::with_capacity(divisions + 1);
                    for ck in 0..=divisions {
                        if let Some(id) = boundary_id(ci, ck) {
                            if let Some(&pco) = out_p.get(id) {
                                row.push(pco);
                            }
                        }
                    }
                    profile_pts.push(row);
                }
                let corner_normal = face_n_dense.get(pi).copied().unwrap_or(Vec3::Y);

                let (cutoff_points, cutoff_polys) = cutoff::build_cutoff_vmesh(
                    &boundary_pts,
                    &profile_pts,
                    corner_pos,
                    corner_normal,
                    divisions,
                );

                // Add cutoff geometry to output
                let cutoff_base = out_p.len();
                for pt in cutoff_points {
                    out_p.push(pt);
                }
                for poly in cutoff_polys {
                    let mapped: Vec<usize> = poly.iter().map(|&i| cutoff_base + i).collect();
                    out_polys.push(mapped);
                    out_poly_src_face.push(PrimId::INVALID);
                    out_poly_corner_n.push(Vec::new());
                }
            } else {
                let boundary_id = |ci: usize, ck: usize| -> Option<usize> {
                    arc_for.get(ci).and_then(|o| *o).and_then(|(s, fwd)| {
                        let he = *spokes_all.get(s)?;
                        let pr = spoke_profiles.get(&he)?;
                        let k = if fwd {
                            ck
                        } else {
                            divisions.saturating_sub(ck)
                        };
                        pr.ids.get(k).copied()
                    })
                };
                emit_vmesh_faces(
                    &vm1,
                    divisions,
                    &mut out_p,
                    &add_point,
                    &boundary_id,
                    &mut out_polys,
                );
                out_poly_src_face.resize(out_polys.len(), PrimId::INVALID);
                out_poly_corner_n.resize(out_polys.len(), Vec::new());
            }
        }

        // 5) [FACE_REBUILD] Rebuild original faces with new boundary vertices (Blender bev_rebuild_polygon).
        // This is the authoritative source of rebuilt original faces: we emit it AFTER bevel strips + corner vmesh.
        for (&prim_id, &start) in &self.topo.primitive_to_halfedge {
            if !start.is_valid() {
                continue;
            }
            let mut vids = Vec::new();
            let mut curr = start;
            loop {
                if let Some(pi) = get_corner_pt(curr, &corner_pt) {
                    vids.push(pi);
                }
                curr = self.topo.next(curr);
                if curr == start {
                    break;
                }
            }
            if vids.len() >= 3 {
                let nn = {
                    let mut n = Vec3::ZERO;
                    for i in 0..vids.len() {
                        let p0 = out_p[vids[i]];
                        let p1 = out_p[vids[(i + 1) % vids.len()]];
                        let p2 = out_p[vids[(i + 2) % vids.len()]];
                        n += (p1 - p0).cross(p2 - p0);
                    }
                    n.normalize_or_zero()
                };
                let on = face_n_of(prim_id, &face_n_dense, self.geo).normalize_or_zero();
                if !self.params.debug_disable_face_rebuild_orient
                    && nn.length_squared() > 1e-12
                    && on.length_squared() > 1e-12
                    && nn.dot(on) < 0.0
                {
                    vids.reverse();
                }
                out_polys.push(vids);
                out_poly_src_face.push(prim_id);
                out_poly_corner_n.push(Vec::new());
            }
        }

        // Build mapping: output point index -> originating original edge (as dense point index pair).
        let mut pt_src_edge: Vec<Option<(usize, usize)>> = vec![None; out_p.len()];
        if divisions > 1 {
            for canon_idx in 0..bev_edge.len() {
                if !bev_edge[canon_idx] {
                    continue;
                }
                let he = dense_to_he
                    .get(canon_idx)
                    .copied()
                    .unwrap_or(HalfEdgeId::INVALID);
                if !he.is_valid() {
                    continue;
                }
                let pair = self.topo.pair(he);
                if !pair.is_valid() {
                    continue;
                }
                let Some(hen) = self.topo.half_edges.get(he.into()) else {
                    continue;
                };
                let u = hen.origin_point;
                let v = self.topo.dest_point(he);
                let (Some(udi), Some(vdi)) = (
                    self.geo.points().get_dense_index(u.into()),
                    self.geo.points().get_dense_index(v.into()),
                ) else {
                    continue;
                };
                let key = if udi <= vdi { (udi, vdi) } else { (vdi, udi) };
                for &eh in [he, pair].iter() {
                    let Some(pr) = spoke_profiles.get(&eh) else {
                        continue;
                    };
                    if pr.ids.len() != divisions + 1 {
                        continue;
                    }
                    for &pid in pr.ids.iter().skip(1).take(divisions - 1) {
                        if pid < pt_src_edge.len() {
                            pt_src_edge[pid] = Some(key);
                        }
                    }
                }
            }
        }

        // Build output Geometry from (out_p, out_polys) with loop-normal semantics via vertex splitting:
        // one GeoVertex per polygon corner, each corner can have its own @N (Blender-style loop normals).
        if self.params.debug_flip_output_winding {
            for (pi, poly) in out_polys.iter_mut().enumerate() {
                poly.reverse();
                if let Some(cn) = out_poly_corner_n.get_mut(pi) {
                    if cn.len() == poly.len() {
                        cn.reverse();
                    }
                }
            }
        }
        let mut g = Geometry::new();
        for _ in 0..out_p.len() {
            let _ = g.points_mut().insert(());
        }
        g.insert_point_attribute("@P", Attribute::new(out_p));

        let poly_normal = |poly: &[usize], pts: &[Vec3]| -> Vec3 {
            if poly.len() < 3 {
                return Vec3::Y;
            }
            let mut n = Vec3::ZERO;
            for i in 0..poly.len() {
                let p0 = pts[poly[i]];
                let p1 = pts[poly[(i + 1) % poly.len()]];
                n.x += (p0.y - p1.y) * (p0.z + p1.z);
                n.y += (p0.z - p1.z) * (p0.x + p1.x);
                n.z += (p0.x - p1.x) * (p0.y + p1.y);
            }
            n.normalize_or_zero()
        };
        let in_mat = self
            .geo
            .get_primitive_attribute(attrs::SHOP_MATERIALPATH)
            .and_then(|a| a.as_slice::<i32>());
        let mut out_mat: Vec<i32> = Vec::new();
        let mut out_strength: Vec<i32> = Vec::new();
        let mut out_pn: Vec<Vec3> = Vec::new();
        let mut out_vn: Vec<Vec3> = Vec::new();
        for (pi, poly) in out_polys.into_iter().enumerate() {
            if poly.len() < 3 {
                continue;
            }
            let src_face = out_poly_src_face
                .get(pi)
                .copied()
                .unwrap_or(PrimId::INVALID);
            let is_inset = src_face.is_valid();
            let fn_inset = if is_inset {
                face_n_of(src_face, &face_n_dense, self.geo).normalize_or_zero()
            } else {
                Vec3::ZERO
            };
            let fn_geom = poly_normal(&poly, g.get_point_position_attribute().unwrap_or(&[]));
            let fn_use = if self.params.harden_normals && is_inset {
                fn_inset
            } else if self.params.harden_normals {
                fn_geom
            } else {
                fn_geom
            };
            let mut verts: Vec<VertexId> = Vec::with_capacity(poly.len());
            let cn = out_poly_corner_n.get(pi).filter(|v| v.len() == poly.len());
            for (ci, &pidx) in poly.iter().enumerate() {
                let pid = PointId::from_raw(pidx as u32, 0);
                let vid = VertexId::from(g.vertices_mut().insert(GeoVertex { point_id: pid }));
                verts.push(vid);
                out_vn.push(cn.and_then(|v| v.get(ci).copied()).unwrap_or(fn_use));
            }
            if verts.len() >= 3 {
                let _ = g
                    .primitives_mut()
                    .insert(GeoPrimitive::Polygon(PolygonPrim { vertices: verts }));
                out_pn.push(fn_use);
                let mat = if is_inset {
                    self.geo
                        .primitives()
                        .get_dense_index(src_face.into())
                        .and_then(|di| in_mat.and_then(|m| m.get(di).copied()))
                        .unwrap_or(0)
                } else if self.params.material >= 0 {
                    self.params.material
                } else {
                    0
                };
                out_mat.push(mat);
                let strong = 2i32;
                let s = match self.params.face_strength {
                    super::structures::FaceStrength::None => 0,
                    super::structures::FaceStrength::New => {
                        if is_inset {
                            0
                        } else {
                            strong
                        }
                    }
                    super::structures::FaceStrength::Affected => strong,
                    super::structures::FaceStrength::All => strong,
                };
                out_strength.push(s);
            }
        }
        if !out_vn.is_empty() {
            g.insert_vertex_attribute(attrs::N, Attribute::new(out_vn));
        }
        if !out_pn.is_empty() {
            g.insert_primitive_attribute(attrs::N, Attribute::new(out_pn));
        }

        // Edge flags (Blender: mark_seam / mark_sharp) + propagation from input edge groups.
        let in_seam = self.geo.get_edge_group("__cunning.seam");
        let in_sharp = self.geo.get_edge_group("__cunning.sharp");
        if self.params.mark_seam
            || self.params.mark_sharp
            || in_seam.is_some()
            || in_sharp.is_some()
        {
            let mut orig_flags: std::collections::HashMap<(usize, usize), (bool, bool)> =
                std::collections::HashMap::new();
            if in_seam.is_some() || in_sharp.is_some() {
                for (ei, e) in self.geo.edges().iter().enumerate() {
                    let (Some(a), Some(b)) = (
                        self.geo.points().get_dense_index(e.p0.into()),
                        self.geo.points().get_dense_index(e.p1.into()),
                    ) else {
                        continue;
                    };
                    let key = if a <= b { (a, b) } else { (b, a) };
                    let seam = in_seam.map(|m| m.get(ei)).unwrap_or(false);
                    let sharp = in_sharp.map(|m| m.get(ei)).unwrap_or(false);
                    if seam || sharp {
                        orig_flags.insert(key, (seam, sharp));
                    }
                }
            }

            let ec = crate::libs::geometry::edge_cache::EdgeCache::build(&g);
            let mut out_seam: Vec<bool> = Vec::with_capacity(ec.edges.len());
            let mut out_sharp: Vec<bool> = Vec::with_capacity(ec.edges.len());
            for (p0, p1) in ec.edges.iter().copied() {
                let i0 = p0.index as usize;
                let i1 = p1.index as usize;
                let is_new = i0 >= n_points || i1 >= n_points;
                let key = if i0 < n_points && i1 < n_points {
                    Some(if i0 <= i1 { (i0, i1) } else { (i1, i0) })
                } else {
                    match (
                        pt_src_edge.get(i0).and_then(|v| *v),
                        pt_src_edge.get(i1).and_then(|v| *v),
                    ) {
                        (Some(k0), Some(k1)) if k0 == k1 => Some(k0),
                        _ => None,
                    }
                };
                let (mut seam, mut sharp) = key
                    .and_then(|k| orig_flags.get(&k).copied())
                    .unwrap_or((false, false));
                if self.params.mark_seam && is_new {
                    seam = true;
                }
                if self.params.mark_sharp && is_new {
                    sharp = true;
                }
                out_seam.push(seam);
                out_sharp.push(sharp);
            }

            for (p0, p1) in ec.edges.iter().copied() {
                let _ = g.add_edge(p0, p1);
            }
            if self.params.mark_seam || in_seam.is_some() {
                let m = g.ensure_edge_group("__cunning.seam");
                for (i, &b) in out_seam.iter().enumerate() {
                    if b {
                        m.set(i, true);
                    }
                }
            }
            if self.params.mark_sharp || in_sharp.is_some() {
                let m = g.ensure_edge_group("__cunning.sharp");
                for (i, &b) in out_sharp.iter().enumerate() {
                    if b {
                        m.set(i, true);
                    }
                }
            }
        }

        // Material + face strength (Blender: mat / face_strength_mode).
        if !out_mat.is_empty() {
            g.insert_primitive_attribute(attrs::SHOP_MATERIALPATH, Attribute::new(out_mat));
            g.insert_detail_attribute(
                attrs::MAT_BY,
                Attribute::new(vec![attrs::SHOP_MATERIALPATH.to_string()]),
            );
        }
        if !out_strength.is_empty()
            && !matches!(
                self.params.face_strength,
                super::structures::FaceStrength::None
            )
        {
            g.insert_primitive_attribute("__cunning.face_strength", Attribute::new(out_strength));
        }
        self.new_geo = g;

        // Return workspace buffers for reuse next time
        WORKSPACE.with(|ws| {
            let mut ws = ws.borrow_mut();
            ws.bev_edge = bev_edge;
            ws.any_bev_point = any_bev_point;
            ws.corner_pt = corner_pt;
            ws.face_normals_dense = face_n_dense;
            ws.dense_to_pid = dense_to_pid;
            ws.dense_to_he = dense_to_he;
            ws.reset(); // Reset while keeping capacity
        });
    }

    fn compute_face_normal(&self, prim_id: PrimId) -> Vec3 {
        let Some(prim) = self.geo.primitives().get(prim_id.into()) else {
            return Vec3::Y;
        };
        let GeoPrimitive::Polygon(poly) = prim else {
            return Vec3::Y;
        };
        if poly.vertices.len() < 3 {
            return Vec3::Y;
        }

        let getp = |vid: VertexId| -> Vec3 {
            self.geo
                .vertices()
                .get(vid.into())
                .map(|v| self.get_pos(v.point_id))
                .unwrap_or(Vec3::ZERO)
        };

        // Keep the original algorithmic intent: accumulate across the loop for robustness.
        let mut n = Vec3::ZERO;
        for i in 0..poly.vertices.len() {
            let p0 = getp(poly.vertices[i]);
            let p1 = getp(poly.vertices[(i + 1) % poly.vertices.len()]);
            let p2 = getp(poly.vertices[(i + 2) % poly.vertices.len()]);
            n += (p1 - p0).cross(p2 - p0);
        }
        n.normalize_or_zero()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::libs::geometry::ids::AttributeId;

    fn cube() -> Geometry {
        let mut g = Geometry::new();
        for _ in 0..8 {
            let _ = g.points_mut().insert(());
        }
        let pts = vec![
            Vec3::new(-1.0, -1.0, -1.0),
            Vec3::new(1.0, -1.0, -1.0),
            Vec3::new(1.0, 1.0, -1.0),
            Vec3::new(-1.0, 1.0, -1.0),
            Vec3::new(-1.0, -1.0, 1.0),
            Vec3::new(1.0, -1.0, 1.0),
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(-1.0, 1.0, 1.0),
        ];
        g.insert_point_attribute(attrs::P, Attribute::new(pts));
        let pids: Vec<PointId> = (0..8).map(|i| PointId::from(i)).collect();
        let mut vids: Vec<VertexId> = Vec::with_capacity(8);
        for pid in pids {
            vids.push(VertexId::from(
                g.vertices_mut().insert(GeoVertex { point_id: pid }),
            ));
        }
        let face = |a: usize, b: usize, c: usize, d: usize| {
            GeoPrimitive::Polygon(PolygonPrim {
                vertices: vec![vids[a], vids[b], vids[c], vids[d]],
            })
        };
        let faces = vec![
            face(0, 1, 2, 3),
            face(4, 5, 6, 7),
            face(0, 4, 5, 1),
            face(1, 5, 6, 2),
            face(2, 6, 7, 3),
            face(3, 7, 4, 0),
        ];
        for f in faces {
            let _ = g.primitives_mut().insert(f);
        }
        g
    }

    #[test]
    fn polybevel_single_face_cube_no_degenerate() {
        let mut g = cube();
        g.ensure_primitive_group("sel");
        if let Some(m) = g.get_primitive_group_mut("sel") {
            if m.len() > 0 {
                m.set(0, true);
            }
        }
        let topo = g.get_topology();
        let mut edge_sel = vec![false; topo.half_edges.len()];
        let pg = g.get_primitive_group("sel").unwrap();
        for (he_idx, he) in topo.half_edges.iter_enumerated() {
            let he_id = HalfEdgeId::from(he_idx);
            let pair = topo.pair(he_id);
            if !pair.is_valid() {
                continue;
            }
            let pa = g
                .primitives()
                .get_dense_index(he.primitive_index.into())
                .map(|di| pg.get(di))
                .unwrap_or(false);
            let pb = topo
                .half_edges
                .get(pair.into())
                .and_then(|p| g.primitives().get_dense_index(p.primitive_index.into()))
                .map(|di| pg.get(di))
                .unwrap_or(false);
            if !(pa || pb) {
                continue;
            }
            if let Some(di) = topo.half_edges.get_dense_index(he_idx) {
                if di < edge_sel.len() {
                    edge_sel[di] = true;
                }
            }
            if let Some(di) = topo.half_edges.get_dense_index(pair.into()) {
                if di < edge_sel.len() {
                    edge_sel[di] = true;
                }
            }
        }
        let builder = super::super::builder::BevelBuilder::new(&g, topo.as_ref());
        let graph = builder.build(&edge_sel, 3, 0.2, None, None);
        let bp = BevelParams::from_node_params(
            0,
            0,
            0.2,
            3,
            0,
            0.5,
            String::new(),
            0,
            0,
            0.1,
            true,
            true,
            false,
            String::new(),
            false,
            false,
            false,
            false,
            false,
            0,
            0,
            -1,
            false,
            1.0,
            0,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
        );
        let out = BevelPipeline::new_with_params(&g, topo.as_ref(), graph, bp).execute();
        assert!(out.points().len() > 8);
        let mut any_bad = false;
        for prim in out.primitives().iter() {
            let GeoPrimitive::Polygon(p) = prim else {
                continue;
            };
            if p.vertices.len() < 3 {
                any_bad = true;
                break;
            }
        }
        assert!(!any_bad);
        let _ = AttributeId::from("@P");
    }

    fn two_quads_plane() -> Geometry {
        let mut g = Geometry::new();
        for _ in 0..6 {
            let _ = g.points_mut().insert(());
        }
        g.insert_point_attribute(
            attrs::P,
            Attribute::new(vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(2.0, 1.0, 0.0),
            ]),
        );
        let pids: Vec<PointId> = (0..6).map(PointId::from).collect();
        let vids: Vec<VertexId> = pids
            .into_iter()
            .map(|pid| VertexId::from(g.vertices_mut().insert(GeoVertex { point_id: pid })))
            .collect();
        let face = |a: usize, b: usize, c: usize, d: usize| {
            GeoPrimitive::Polygon(PolygonPrim {
                vertices: vec![vids[a], vids[b], vids[c], vids[d]],
            })
        };
        for f in [face(0, 1, 4, 3), face(1, 2, 5, 4)] {
            let _ = g.primitives_mut().insert(f);
        }
        g
    }

    #[test]
    fn polybevel_boundary_vertex_spokes_do_not_flip_winding() {
        let g = two_quads_plane();
        let topo = g.get_topology();
        let mut edge_sel = vec![false; topo.half_edges.len()];
        // Select the shared interior edge by selecting both faces (boundary edges have no pair and are ignored).
        for (he_idx, _) in topo.half_edges.iter_enumerated() {
            let he = HalfEdgeId::from(he_idx);
            let pair = topo.pair(he);
            if !pair.is_valid() {
                continue;
            }
            if let Some(di) = topo.half_edges.get_dense_index(he_idx) {
                edge_sel[di] = true;
            }
            if let Some(di) = topo.half_edges.get_dense_index(pair.into()) {
                edge_sel[di] = true;
            }
        }
        let builder = super::super::builder::BevelBuilder::new(&g, topo.as_ref());
        let graph = builder.build(&edge_sel, 2, 0.1, None, None);
        let bp = BevelParams::from_node_params(
            0, 0, 0.1, 2, 0, 0.5, String::new(), 0, 0, 0.1, true, true, false, String::new(),
            false, false, false, false, false, 0, 0, -1, false, 1.0, 0, false, false, false, false,
            false, false, false, false, false, false, false, false, false, false, false,
        );
        let out = BevelPipeline::new_with_params(&g, topo.as_ref(), graph, bp).execute();
        let pts = out.get_point_position_attribute().unwrap_or(&[]);
        let poly_n = |poly: &PolygonPrim| -> Vec3 {
            let ids: Vec<Vec3> = poly
                .vertices
                .iter()
                .filter_map(|&vid| out.vertices().get(vid.into()).map(|v| v.point_id))
                .filter_map(|pid| out.points().get_dense_index(pid.into()).and_then(|di| pts.get(di).copied()))
                .collect();
            if ids.len() < 3 {
                return Vec3::ZERO;
            }
            let mut n = Vec3::ZERO;
            for i in 0..ids.len() {
                let p0 = ids[i];
                let p1 = ids[(i + 1) % ids.len()];
                n.x += (p0.y - p1.y) * (p0.z + p1.z);
                n.y += (p0.z - p1.z) * (p0.x + p1.x);
                n.z += (p0.x - p1.x) * (p0.y + p1.y);
            }
            n.normalize_or_zero()
        };
        // Expect all faces to keep consistent +Z winding for this coplanar input.
        for prim in out.primitives().iter() {
            let GeoPrimitive::Polygon(p) = prim else { continue };
            let n = poly_n(p);
            if n.length_squared() > 1e-10 {
                assert!(n.dot(Vec3::Z) >= 0.0);
            }
        }
    }
}
