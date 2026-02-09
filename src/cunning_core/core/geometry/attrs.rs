/// Built-in attribute names (Houdini-compatible).
/// Using standard Houdini convention: P, N, uv, Cd, etc.
/// Note: Cunning3D stores names with "@" prefix internally for legacy reasons, 
/// but we expose them as constants to avoid magic strings.

pub const P: &str = "@P";
pub const N: &str = "@N";
pub const UV: &str = "@uv";
pub const UV2: &str = "@uv2";
pub const CD: &str = "@Cd";        // Diffuse Color
pub const ALPHA: &str = "@Alpha";  // Alpha transparency
pub const V: &str = "@v";          // Velocity
pub const FORCE: &str = "@force";  // Force
pub const MASS: &str = "@mass";    // Mass
pub const ID: &str = "@id";        // Stable Element ID (if explicit)
pub const NAME: &str = "@name";    // Primitive Name / Path

// Intrinsic / Computed attributes
pub const PRIM_AREA: &str = "@area";
pub const PRIM_PERIMETER: &str = "@perimeter";
pub const PT_NEIGHBORS: &str = "@neighbors"; // Array of neighbor IDs

// Curve (Unity Spline compatible)
pub const KNOT_TIN: &str = "@knot_tangent_in";
pub const KNOT_TOUT: &str = "@knot_tangent_out";
pub const KNOT_ROT: &str = "@knot_rot";
pub const KNOT_MODE: &str = "@knot_mode";
pub const KNOT_TENSION: &str = "@knot_tension";
pub const KNOT_LINK_ID: &str = "@knot_link_id";

// Resample / Curve analysis
pub const CURVEU: &str = "@curveu";
pub const TANGENTU: &str = "@tangentu";
pub const TANGENTV: &str = "@tangentv";

// Material (DCC only; compatible with Bevy PBR / StandardMaterial semantics)
pub const MAT_KIND: &str = "__cunning.mat.kind";                 // String[1]
pub const MAT_ID: &str = "__cunning.mat.id";                     // String[1]
pub const MAT_BY: &str = "__cunning.mat.by";                     // String[1] (primitive attr name)
pub const MAT_BASECOLOR_TEX: &str = "__cunning.mat.basecolor_tex"; // String[1]
pub const MAT_NORMAL_TEX: &str = "__cunning.mat.normal_tex";       // String[1]
pub const MAT_ORM_TEX: &str = "__cunning.mat.orm_tex";             // String[1]  (R=AO, G=Roughness, B=Metallic)
pub const MAT_BASECOLOR_TINT: &str = "__cunning.mat.basecolor_tint"; // Vec4[1] (rgba, sRGB intent)
pub const MAT_ROUGHNESS: &str = "__cunning.mat.roughness";         // f32[1]
pub const MAT_METALLIC: &str = "__cunning.mat.metallic";           // f32[1]
pub const MAT_EMISSIVE: &str = "__cunning.mat.emissive";           // Vec3[1] (linear rgb)
pub const MAT_EMISSIVE_TEX: &str = "__cunning.mat.emissive_tex";   // String[1]

// Houdini-compatible primitive material assignment key.
pub const SHOP_MATERIALPATH: &str = "@shop_materialpath"; // String[prim]