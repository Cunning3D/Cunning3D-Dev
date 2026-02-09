use crate::libs::geometry::geo_ref::ForEachMeta;
use std::cell::RefCell;

thread_local! { static CTX: RefCell<Vec<(String, ForEachMeta)>> = const { RefCell::new(Vec::new()) }; }

#[inline]
pub fn push(block_id: String, meta: ForEachMeta) {
    CTX.with(|c| c.borrow_mut().push((block_id, meta)));
}
#[inline]
pub fn pop() {
    CTX.with(|c| {
        let _ = c.borrow_mut().pop();
    });
}
#[inline]
pub fn last() -> Option<(String, ForEachMeta)> {
    CTX.with(|c| c.borrow().last().cloned())
}
#[inline]
pub fn find_rev(block_id: &str) -> Option<ForEachMeta> {
    CTX.with(|c| {
        c.borrow()
            .iter()
            .rev()
            .find(|(b, _)| b == block_id)
            .map(|(_, m)| m.clone())
    })
}
