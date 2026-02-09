//! Custom Profile support: allows user-defined bevel profile curves.
//! Port of Blender's CurveProfile-based profile sampling (profile_type == BEVEL_PROFILE_CUSTOM).
use bevy::prelude::*;

/// Custom profile data: a sequence of normalized (0-1) sample points.
/// x = distance along profile (0 = start, 1 = end)
/// y = depth/height of profile at that point (0 = outer edge, 1 = inner/middle)
#[derive(Clone, Debug)]
pub struct CustomProfile {
    /// Sample points normalized to [0,1] x [0,1] space
    pub samples: Vec<Vec2>,
    /// Cached arc-length parameterization for even spacing
    pub arc_lengths: Vec<f32>,
    /// Total arc length
    pub total_length: f32,
}

impl Default for CustomProfile {
    fn default() -> Self {
        // Default: quarter circle from (0,1) -> (1,0) in unit square (matches superellipse basis).
        let mut samples = Vec::with_capacity(9);
        for i in 0..=8 {
            let t = i as f32 / 8.0;
            let angle = t * std::f32::consts::FRAC_PI_2;
            samples.push(Vec2::new(angle.sin(), angle.cos()));
        }
        let mut profile = Self {
            samples,
            arc_lengths: Vec::new(),
            total_length: 0.0,
        };
        profile.compute_arc_lengths();
        profile
    }
}

impl CustomProfile {
    /// Create a custom profile from sample points.
    pub fn new(samples: Vec<Vec2>) -> Self {
        let mut profile = Self {
            samples,
            arc_lengths: Vec::new(),
            total_length: 0.0,
        };
        profile.compute_arc_lengths();
        profile
    }

    /// Create a preset profile: "Concave" (inward curve)
    pub fn concave() -> Self {
        let samples: Vec<Vec2> = (0..=8)
            .map(|i| {
                let t = i as f32 / 8.0;
                let angle = t * std::f32::consts::FRAC_PI_2;
                Vec2::new(t, 1.0 - angle.cos()) // Concave curve
            })
            .collect();
        Self::new(samples)
    }

    /// Create a preset profile: "Convex" (outward curve, like default circle)
    pub fn convex() -> Self {
        Self::default()
    }

    /// Create a preset profile: "Steps" (stair-step pattern)
    pub fn steps(n_steps: usize) -> Self {
        let mut samples = Vec::with_capacity(n_steps * 2 + 2);
        samples.push(Vec2::ZERO);
        for i in 0..n_steps {
            let t = i as f32 / n_steps as f32;
            let t_next = (i + 1) as f32 / n_steps as f32;
            let h = (i + 1) as f32 / n_steps as f32;
            samples.push(Vec2::new(t, h));
            samples.push(Vec2::new(t_next, h));
        }
        samples.push(Vec2::ONE);
        Self::new(samples)
    }

    /// Compute arc lengths for even parameterization.
    fn compute_arc_lengths(&mut self) {
        self.arc_lengths.clear();
        self.arc_lengths.push(0.0);

        let mut total = 0.0;
        for i in 1..self.samples.len() {
            let d = (self.samples[i] - self.samples[i - 1]).length();
            total += d;
            self.arc_lengths.push(total);
        }
        self.total_length = total;
    }

    /// Sample the profile at parameter t in [0, 1].
    /// Returns (x, y) where x is horizontal distance, y is vertical depth.
    pub fn sample(&self, t: f32) -> Vec2 {
        if self.samples.is_empty() {
            return Vec2::new(t, t);
        }
        if self.samples.len() == 1 {
            return self.samples[0];
        }

        let t = t.clamp(0.0, 1.0);

        // Find the segment containing t
        for i in 1..self.samples.len() {
            let t0 = self.samples[i - 1].x;
            let t1 = self.samples[i].x;
            if t <= t1 || i == self.samples.len() - 1 {
                let local_t = if (t1 - t0).abs() < 1e-6 {
                    0.0
                } else {
                    (t - t0) / (t1 - t0)
                };
                return self.samples[i - 1].lerp(self.samples[i], local_t);
            }
        }

        *self.samples.last().unwrap()
    }

    /// Sample at even arc-length intervals.
    pub fn sample_arc_length(&self, t: f32) -> Vec2 {
        if self.total_length < 1e-6 {
            return self.sample(t);
        }

        let target_len = t * self.total_length;

        // Find segment containing target_len
        for i in 1..self.arc_lengths.len() {
            if target_len <= self.arc_lengths[i] || i == self.arc_lengths.len() - 1 {
                let len0 = self.arc_lengths[i - 1];
                let len1 = self.arc_lengths[i];
                let local_t = if (len1 - len0).abs() < 1e-6 {
                    0.0
                } else {
                    (target_len - len0) / (len1 - len0)
                };
                return self.samples[i - 1].lerp(self.samples[i], local_t);
            }
        }

        *self.samples.last().unwrap()
    }
}

/// Apply custom profile to generate 3D profile points between start and end.
pub fn apply_custom_profile(
    profile: &CustomProfile,
    start: Vec3,
    end: Vec3,
    middle: Vec3, // The vertex being beveled (determines depth direction)
    segments: usize,
) -> Vec<Vec3> {
    let mut result = Vec::with_capacity(segments + 1);

    // Compute profile axes
    let edge_dir = (end - start).normalize_or_zero();
    let to_middle = (middle - start).normalize_or_zero();
    let depth_dir = to_middle - edge_dir * edge_dir.dot(to_middle); // Perpendicular component
    let depth_dir = depth_dir.normalize_or_zero();
    let max_depth = (middle - start).length() * 0.5; // Half distance to middle

    for k in 0..=segments {
        let t = k as f32 / segments as f32;
        let sample = profile.sample_arc_length(t);

        // sample.x = position along edge
        // sample.y = depth toward middle
        let edge_pos = start.lerp(end, sample.x);
        let depth_offset = depth_dir * (sample.y * max_depth);

        result.push(edge_pos + depth_offset);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_profile() {
        let profile = CustomProfile::default();
        assert!(profile.samples.len() >= 2);

        let start = profile.sample(0.0);
        let end = profile.sample(1.0);
        assert!(start.x < 0.01);
        assert!((end.x - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_steps_profile() {
        let profile = CustomProfile::steps(3);
        assert!(profile.samples.len() >= 4);
    }

    #[test]
    fn test_apply_custom_profile() {
        let profile = CustomProfile::default();
        let start = Vec3::new(0.0, 0.0, 0.0);
        let end = Vec3::new(1.0, 0.0, 0.0);
        let middle = Vec3::new(0.5, 0.5, 0.0);

        let result = apply_custom_profile(&profile, start, end, middle, 4);
        assert_eq!(result.len(), 5);
        assert!((result[0] - start).length() < 0.1);
    }
}
