pub trait ShelfDefinition: Send + Sync {
    fn name(&self) -> &str;
    // TODO: Define Tool structure
}
