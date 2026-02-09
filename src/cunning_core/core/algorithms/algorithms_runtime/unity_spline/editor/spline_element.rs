use super::super::unity_spline::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectableElement {
    Knot(SelectableKnot),
    Tangent(SelectableTangent),
}

impl SelectableElement {
    #[inline] pub fn knot_index(&self) -> usize { match self { Self::Knot(k) => k.knot_index, Self::Tangent(t) => t.knot_index } }
    #[inline] pub fn spline_index(&self) -> usize { match self { Self::Knot(k) => k.spline_index, Self::Tangent(t) => t.spline_index } }
}
