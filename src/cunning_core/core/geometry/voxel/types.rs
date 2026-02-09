use serde::{Deserialize, Serialize};

/// Default chunk size for discrete voxels.
///
/// - 16 is a good baseline (cache-friendly, common in voxel engines).
/// - Keep in sync with meshing code assumptions.
pub const CHUNK_SIZE: i32 = 16;
pub const CHUNK_SIZE_USIZE: usize = CHUNK_SIZE as usize;

/// Palette index type (0 = empty).
pub type VoxelPi = u8;

/// A single voxel value (palette + optional override).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Voxel {
    pub palette_index: VoxelPi,
    pub color_override: Option<[u8; 4]>,
}

impl Default for Voxel {
    fn default() -> Self {
        Self {
            palette_index: 0,
            color_override: None,
        }
    }
}

/// Material-ish palette entry (kept simple; extend as needed).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PaletteEntry {
    pub color: [u8; 4],
    #[serde(default)]
    pub roughness: f32,
    #[serde(default)]
    pub metallic: f32,
    #[serde(default)]
    pub emissive: f32,
}

impl Default for PaletteEntry {
    fn default() -> Self {
        Self {
            color: [255, 255, 255, 255],
            roughness: 0.5,
            metallic: 0.0,
            emissive: 0.0,
        }
    }
}

pub type VoxelPalette = Vec<PaletteEntry>;

