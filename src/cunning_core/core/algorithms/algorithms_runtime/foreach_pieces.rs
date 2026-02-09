use std::collections::HashMap;
use std::sync::Arc;
use crate::libs::geometry::geo_ref::{ForEachMeta, GeometryView};
use crate::libs::geometry::group::ElementGroupMask;
use crate::libs::geometry::mesh::Geometry;

#[derive(Clone)]
pub enum ForeachPiecePlanItem { FullInput, View(Arc<GeometryView>) }

#[derive(Clone)]
pub struct ForeachPiecePlanParams { pub domain: i32, pub method: i32, pub attr: String, pub count: usize }

#[inline]
fn mk_view(base: Arc<Geometry>, prim: Option<&ElementGroupMask>, pts: Option<&ElementGroupMask>, m: ForEachMeta) -> ForeachPiecePlanItem {
    ForeachPiecePlanItem::View(Arc::new(GeometryView::from_masks(base, prim, pts, Some(m))))
}

pub fn plan_foreach_pieces(base: Arc<Geometry>, p: ForeachPiecePlanParams) -> Vec<ForeachPiecePlanItem> {
    let count = p.count.max(1);
    if p.method == 1 { return (0..count).map(|_| ForeachPiecePlanItem::FullInput).collect(); }
    let a = p.attr.trim();
    let a_norm = a.trim_start_matches('@');

    if p.domain == 1 {
        let n = base.points().len();
        if a_norm.eq_ignore_ascii_case("ptnum") || a_norm.eq_ignore_ascii_case("pointnum") {
            return (0..n).map(|it| { let mut m = ElementGroupMask::new(n); m.set(it, true); mk_view(base.clone(), None, Some(&m), ForEachMeta { iteration: it as i32, numiterations: n as i32, value: it.to_string(), ivalue: it as i32 }) }).collect();
        }
        if let Some(a) = base.get_point_attribute(a_norm).and_then(|a| a.as_slice::<i32>()) {
            let mut mm: HashMap<i32, Vec<usize>> = HashMap::new();
            for (i, v) in a.iter().enumerate() { mm.entry(*v).or_default().push(i); }
            let mut ks: Vec<i32> = mm.keys().copied().collect(); ks.sort_unstable();
            let len = mm.len() as i32;
            return ks.into_iter().enumerate().map(|(it, k)| { let mut mask = ElementGroupMask::new(n); if let Some(is) = mm.get(&k) { for &i in is { mask.set(i, true); } } mk_view(base.clone(), None, Some(&mask), ForEachMeta { iteration: it as i32, numiterations: len, value: k.to_string(), ivalue: k }) }).collect();
        }
        if let Some(a) = base.get_point_attribute(a_norm).and_then(|a| a.as_slice::<String>()) {
            let mut mm: HashMap<String, Vec<usize>> = HashMap::new();
            for (i, v) in a.iter().enumerate() { mm.entry(v.clone()).or_default().push(i); }
            let mut ks: Vec<String> = mm.keys().cloned().collect(); ks.sort();
            let len = mm.len() as i32;
            return ks.into_iter().enumerate().map(|(it, k)| { let mut mask = ElementGroupMask::new(n); if let Some(is) = mm.get(&k) { for &i in is { mask.set(i, true); } } mk_view(base.clone(), None, Some(&mask), ForEachMeta { iteration: it as i32, numiterations: len, value: k, ivalue: it as i32 }) }).collect();
        }
    } else {
        let n = base.primitives().len();
        if a_norm.eq_ignore_ascii_case("primnum") || a_norm.eq_ignore_ascii_case("primitivenum") {
            return (0..n).map(|it| { let mut m = ElementGroupMask::new(n); m.set(it, true); mk_view(base.clone(), Some(&m), None, ForEachMeta { iteration: it as i32, numiterations: n as i32, value: it.to_string(), ivalue: it as i32 }) }).collect();
        }
        if let Some(a) = base.get_primitive_attribute(a_norm).and_then(|a| a.as_slice::<i32>()) {
            let mut mm: HashMap<i32, Vec<usize>> = HashMap::new();
            for (i, v) in a.iter().enumerate() { mm.entry(*v).or_default().push(i); }
            let mut ks: Vec<i32> = mm.keys().copied().collect(); ks.sort_unstable();
            let len = mm.len() as i32;
            return ks.into_iter().enumerate().map(|(it, k)| { let mut mask = ElementGroupMask::new(n); if let Some(is) = mm.get(&k) { for &i in is { mask.set(i, true); } } mk_view(base.clone(), Some(&mask), None, ForEachMeta { iteration: it as i32, numiterations: len, value: k.to_string(), ivalue: k }) }).collect();
        }
        if let Some(a) = base.get_primitive_attribute(a_norm).and_then(|a| a.as_slice::<String>()) {
            let mut mm: HashMap<String, Vec<usize>> = HashMap::new();
            for (i, v) in a.iter().enumerate() { mm.entry(v.clone()).or_default().push(i); }
            let mut ks: Vec<String> = mm.keys().cloned().collect(); ks.sort();
            let len = mm.len() as i32;
            return ks.into_iter().enumerate().map(|(it, k)| { let mut mask = ElementGroupMask::new(n); if let Some(is) = mm.get(&k) { for &i in is { mask.set(i, true); } } mk_view(base.clone(), Some(&mask), None, ForEachMeta { iteration: it as i32, numiterations: len, value: k, ivalue: it as i32 }) }).collect();
        }
    }
    vec![mk_view(base, None, None, ForEachMeta { iteration: 0, numiterations: 1, value: "0".into(), ivalue: 0 })]
}

