//! Stable port keys (connection keys) decoupled from display names.
//! These are persisted in graphs; rename only affects UI labels, never the key.

use crate::nodes::PortId;

#[inline]
pub fn in0() -> PortId {
    PortId::from("in:0")
}
#[inline]
pub fn in1() -> PortId {
    PortId::from("in:1")
}
#[inline]
pub fn out0() -> PortId {
    PortId::from("out:0")
}
#[inline]
pub fn in_a() -> PortId {
    PortId::from("in:a")
}
#[inline]
pub fn in_b() -> PortId {
    PortId::from("in:b")
}
#[inline]
pub fn feedback_in() -> PortId {
    PortId::from("feedback_in")
}
#[inline]
pub fn feedback_out() -> PortId {
    PortId::from("feedback_out")
}

#[inline]
pub fn is_cda_port_key(k: &PortId) -> bool {
    k.as_str().starts_with("cda:")
}

#[inline]
pub fn is_in0(k: &PortId) -> bool {
    k.as_str() == "in:0"
}
#[inline]
pub fn is_in1(k: &PortId) -> bool {
    k.as_str() == "in:1"
}
#[inline]
pub fn is_out0(k: &PortId) -> bool {
    k.as_str() == "out:0"
}
#[inline]
pub fn is_in_a(k: &PortId) -> bool {
    k.as_str() == "in:a"
}
#[inline]
pub fn is_in_b(k: &PortId) -> bool {
    k.as_str() == "in:b"
}
#[inline]
pub fn is_feedback_in(k: &PortId) -> bool {
    k.as_str() == "feedback_in"
}
#[inline]
pub fn is_feedback_out(k: &PortId) -> bool {
    k.as_str() == "feedback_out"
}

#[inline]
pub fn port_sort_key(k: &PortId) -> (u8, u32) {
    let s = k.as_str();
    let rank = if s.starts_with("in:") {
        0
    } else if s.starts_with("out:") {
        1
    } else {
        2
    };
    let idx = s
        .rsplit_once(':')
        .and_then(|(_, t)| t.parse::<u32>().ok())
        .unwrap_or(u32::MAX);
    (rank, idx)
}
