use crate::libs::geometry::mesh::Geometry;

mod split;
mod collapse;
mod flip;
mod smooth;
mod utils;

pub use split::split_edges;
pub use collapse::collapse_edges;
pub use flip::flip_edges;
pub use smooth::smooth_vertices;

#[derive(Debug, Clone)]
pub struct RemeshOptions {
    pub target_length: f32,
    pub iterations: usize,
    pub smooth_strength: f32,
    pub project_to_original: bool,
}

impl Default for RemeshOptions {
    fn default() -> Self {
        Self {
            target_length: 0.1,
            iterations: 5,
            smooth_strength: 0.5,
            project_to_original: false,
        }
    }
}

pub fn isotropic_remesh(input_geo: &Geometry, options: RemeshOptions) -> Geometry {
    puffin::profile_function!();
    if options.iterations == 0 || options.target_length <= 0.0 || input_geo.primitives().is_empty() {
        return input_geo.clone();
    }
    // 1. Prepare working geometry (mutable clone)
    let mut geo = input_geo.fork();
    
    // Ensure topology is clean first? 
    // Ideally we should triangulate if not already triangulated.
    // For now assume input is triangulated or mostly triangulated.
    
    // We need to rebuild topology at each step or maintain it dynamically.
    // Dynamic maintenance is faster but harder to implement correctly.
    // For simplicity and robustness (as per "Cunning3D" style), we will rebuild topology 
    // implicitly or explicitly where needed, but keeping it "stateless" between passes 
    // is safer for a first implementation, though slower. 
    
    // However, the standard algorithm relies on a valid half-edge structure.
    // Let's attach topology to a context.
    
    for it in 0..options.iterations {
        puffin::profile_scope!("remesh_iteration");
        let _ = it;
        // 1. Split long edges
        // Topology changes, so we must rebuild or update. 
        // For MVP: Rebuild topology inside each step if needed, or pass it through.
        // Since our Topology struct is read-only built from Geometry, we update Geometry and rebuild Topology.
        puffin::profile_scope!("remesh_split");
        split_edges(&mut geo, options.target_length);
        
        // 2. Collapse short edges
        puffin::profile_scope!("remesh_collapse");
        collapse_edges(&mut geo, options.target_length);
        
        // 3. Flip edges
        puffin::profile_scope!("remesh_flip");
        flip_edges(&mut geo);
        
        // 4. Smooth
        puffin::profile_scope!("remesh_smooth");
        smooth_vertices(&mut geo, options.smooth_strength);
    }
    
    // Project back to original if requested (skipped for now as we need spatial structure)
    
    // Final normals update
    geo.calculate_smooth_normals(); // or flat
    
    geo
}
