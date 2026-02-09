pub fn mix_rect(key: u64, rect: bevy_egui::egui::Rect) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    rect.min.x.to_bits().hash(&mut h);
    rect.min.y.to_bits().hash(&mut h);
    rect.max.x.to_bits().hash(&mut h);
    rect.max.y.to_bits().hash(&mut h);
    h.finish()
}
