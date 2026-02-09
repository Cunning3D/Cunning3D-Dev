use super::super::unity_spline::*;

/// Equivalent to UnityEditor.Splines.SplineSelectionUtility.IsSelectable(SelectableTangent) core rules.
pub fn is_selectable_tangent(spline: &Spline, knot_index: usize, tangent: BezierTangent) -> bool {
    if knot_index >= spline.count() { return false; }
    if !are_tangents_modifiable(spline.meta[knot_index].mode) { return false; }
    if spline.closed { return true; }
    match tangent { BezierTangent::In => knot_index != 0, BezierTangent::Out => knot_index + 1 != spline.count() }
}

/// Equivalent to UnityEditor.Splines.SplineSelectionUtility.CanSplitSelection for a single knot selection.
pub fn can_split_selection(spline: &Spline, knot_index: usize) -> bool {
    if spline.count() < 2 || knot_index >= spline.count() { return false; }
    let end_knot = knot_index == 0 || knot_index + 1 == spline.count();
    !end_knot || spline.closed
}

pub fn can_unlink_knots(container: &SplineContainer, knots: &[SelectableKnot]) -> bool {
    if knots.is_empty() { return false; }
    for k in knots {
        let links = container.links.get_knot_links(SplineKnotIndex::new(k.spline_index as i32, k.knot_index as i32));
        if links.len() > 1 { return true; }
    }
    false
}

pub fn can_link_knots(container: &SplineContainer, knots: &[SelectableKnot]) -> bool {
    if knots.len() < 2 { return false; }
    for i in 0..knots.len() {
        let a = SplineKnotIndex::new(knots[i].spline_index as i32, knots[i].knot_index as i32);
        let a_links = container.links.get_knot_links(a);
        for j in (i + 1)..knots.len() {
            let b = SplineKnotIndex::new(knots[j].spline_index as i32, knots[j].knot_index as i32);
            if !a_links.contains(&b) { return true; }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec3;
    use super::*;

    fn mk_linear_spline(count: usize, closed: bool) -> Spline {
        let mut s = Spline::default();
        s.knots = (0..count).map(|i| BezierKnot { position: Vec3::Z * i as f32, ..Default::default() }).collect();
        s.meta = vec![MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION); count];
        s.closed = closed;
        s
    }

    #[test]
    fn tangent_not_selectable_on_open_endpoints() {
        let s = mk_linear_spline(3, false);
        assert!(!is_selectable_tangent(&s, 0, BezierTangent::In));
        assert!(!is_selectable_tangent(&s, 2, BezierTangent::Out));
        assert!(is_selectable_tangent(&s, 1, BezierTangent::In));
        assert!(is_selectable_tangent(&s, 1, BezierTangent::Out));
    }

    #[test]
    fn can_split_selection_matches_closed_rule() {
        let open = mk_linear_spline(4, false);
        assert!(!can_split_selection(&open, 0));
        assert!(!can_split_selection(&open, 3));
        assert!(can_split_selection(&open, 1));
        let closed = mk_linear_spline(4, true);
        assert!(can_split_selection(&closed, 0));
        assert!(can_split_selection(&closed, 3));
    }

    #[test]
    fn can_link_unlink_knots_uses_link_collection() {
        let mut c = SplineContainer::default();
        c.splines.push(mk_linear_spline(3, false));
        let a = SelectableKnot { spline_index: 0, knot_index: 0 };
        let b = SelectableKnot { spline_index: 0, knot_index: 2 };
        assert!(can_link_knots(&c, &[a, b]));
        assert!(!can_unlink_knots(&c, &[a, b]));
        c.link_knots(SplineKnotIndex::new(0, 0), SplineKnotIndex::new(0, 2));
        assert!(can_unlink_knots(&c, &[a]));
        assert!(!can_link_knots(&c, &[a, b]));
    }
}
