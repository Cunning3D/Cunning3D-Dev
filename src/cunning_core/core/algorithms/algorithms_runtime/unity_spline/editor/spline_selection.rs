use super::spline_element::SelectableElement;
use super::super::unity_spline::*;

#[derive(Clone, Debug, Default)]
pub struct SplineSelectionState {
    pub selected_elements: Vec<SelectableElement>,
    pub active_element: Option<SelectableElement>,
}

impl SplineSelectionState {
    #[inline] pub fn clear(&mut self) { self.selected_elements.clear(); self.active_element = None; }

    #[inline] pub fn contains(&self, e: SelectableElement) -> bool { self.selected_elements.iter().any(|x| *x == e) }
    #[inline] pub fn is_active(&self, e: SelectableElement) -> bool { self.active_element == Some(e) }
    #[inline] pub fn set_active(&mut self, e: Option<SelectableElement>) { self.active_element = e; }

    pub fn add(&mut self, e: SelectableElement) { if !self.contains(e) { self.selected_elements.push(e); } }
    pub fn remove(&mut self, e: SelectableElement) {
        self.selected_elements.retain(|x| *x != e);
        if self.active_element == Some(e) { self.active_element = None; }
    }
    pub fn add_range(&mut self, es: &[SelectableElement]) { for &e in es { self.add(e); } }
    pub fn remove_range(&mut self, es: &[SelectableElement]) { for &e in es { self.remove(e); } }

    #[inline] pub fn has_active_spline_selection(&self) -> bool { !self.selected_elements.is_empty() }
    #[inline] pub fn contains_spline(&self, spline_index: usize) -> bool { self.selected_elements.iter().any(|e| e.spline_index() == spline_index) }

    /// Unity-style adjacency used for tangent visibility: a tangent is adjacent if its owner knot is selected, etc.
    pub fn is_selected_or_adjacent_to_selected(&self, e: SelectableElement) -> bool {
        if self.contains(e) { return true; }
        match e {
            SelectableElement::Knot(k) => {
                let ti = SelectableElement::Tangent(SelectableTangent { spline_index: k.spline_index, knot_index: k.knot_index, tangent: BezierTangent::In });
                let to = SelectableElement::Tangent(SelectableTangent { spline_index: k.spline_index, knot_index: k.knot_index, tangent: BezierTangent::Out });
                self.contains(ti) || self.contains(to)
            }
            SelectableElement::Tangent(t) => {
                let k = SelectableElement::Knot(SelectableKnot { spline_index: t.spline_index, knot_index: t.knot_index });
                let opp = SelectableElement::Tangent(SelectableTangent { spline_index: t.spline_index, knot_index: t.knot_index, tangent: if t.tangent == BezierTangent::In { BezierTangent::Out } else { BezierTangent::In } });
                self.contains(k) || self.contains(opp)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_add_remove_active() {
        let mut s = SplineSelectionState::default();
        let k = SelectableElement::Knot(SelectableKnot { spline_index: 0, knot_index: 1 });
        s.add(k);
        assert!(s.contains(k));
        s.set_active(Some(k));
        assert!(s.is_active(k));
        s.remove(k);
        assert!(!s.contains(k));
        assert!(!s.has_active_spline_selection());
        assert_eq!(s.active_element, None);
    }

    #[test]
    fn adjacent_rules_knot_tangent() {
        let mut s = SplineSelectionState::default();
        let k = SelectableElement::Knot(SelectableKnot { spline_index: 0, knot_index: 2 });
        let t = SelectableElement::Tangent(SelectableTangent { spline_index: 0, knot_index: 2, tangent: BezierTangent::In });
        s.add(k);
        assert!(s.is_selected_or_adjacent_to_selected(t));
        s.clear();
        s.add(t);
        assert!(s.is_selected_or_adjacent_to_selected(k));
    }
}

