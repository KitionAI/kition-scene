//! Screen-space ambient occlusion for final renders: how much of the sky each visible point
//! sees, estimated from the depth picture. Points in creases and under other surfaces see less,
//! so the world's ambient light reaches them less.

use glam::{Mat4, Vec3};

use crate::view::Projector;

/// Fixed directions in the upper hemisphere (z up), spread and of varied length.
const KERNEL: [[f32; 3]; 12] = [
    [0.53, 0.00, 0.30],
    [-0.27, 0.46, 0.38],
    [-0.27, -0.46, 0.20],
    [0.18, 0.31, 0.72],
    [-0.36, 0.00, 0.55],
    [0.18, -0.31, 0.48],
    [0.80, 0.29, 0.25],
    [-0.65, 0.55, 0.15],
    [-0.15, -0.84, 0.31],
    [0.05, 0.09, 0.95],
    [0.42, -0.62, 0.40],
    [-0.70, -0.32, 0.52],
];

/// The occlusion picture: 1 where a point sees all of the sky, lower where it sees less.
pub struct Occlusion {
    pub width: u32,
    pub height: u32,
    pub factor: Vec<f32>,
}

impl Occlusion {
    /// The factor at a pixel of a picture `scale` times as large (1 where unknown).
    pub fn at(&self, x: u32, y: u32, scale: u32) -> f32 {
        let scale = scale.max(1);
        let (x, y) = (x / scale, y / scale);
        if x >= self.width || y >= self.height {
            return 1.0;
        }
        self.factor.get(y as usize * self.width as usize + x as usize).copied().unwrap_or(1.0)
    }
}

/// Estimates occlusion at `width` × `height` from an eye-depth picture `scale` times as large
/// (seen through `projector`), looking `radius` metres around each point.
pub fn occlusion(depth: &[f32], projector: &Projector, scale: u32, width: u32, height: u32, radius: f32) -> Occlusion {
    let scale = scale.max(1);
    let (big_width, big_height) = (projector.width, projector.height);
    let inverse = projector.projection.inverse();
    // The eye-space point under a big-picture pixel at eye depth `d`.
    let point_at = |x: f32, y: f32, d: f32| -> Vec3 {
        let ndc_x = x / big_width.max(1) as f32 * 2.0 - 1.0;
        let ndc_y = 1.0 - y / big_height.max(1) as f32 * 2.0;
        let near = inverse * glam::Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
        let far = inverse * glam::Vec4::new(ndc_x, ndc_y, 0.5, 1.0);
        let (near, far) = (near.truncate() / near.w, far.truncate() / far.w);
        let direction = far - near;
        if direction.z.abs() < 1e-12 {
            return near;
        }
        let t = (-d - near.z) / direction.z;
        near + direction * t
    };
    let depth_at = |x: i64, y: i64| -> f32 {
        if x < 0 || y < 0 || x >= i64::from(big_width) || y >= i64::from(big_height) {
            return f32::INFINITY;
        }
        depth.get(y as usize * big_width as usize + x as usize).copied().unwrap_or(f32::INFINITY)
    };
    let mut raw = vec![1.0f32; (width as usize).saturating_mul(height as usize)];
    let projection: Mat4 = projector.projection;
    for y in 0..height {
        for x in 0..width {
            let (bx, by) = (x * scale + scale / 2, y * scale + scale / 2);
            let d = depth_at(i64::from(bx), i64::from(by));
            if !d.is_finite() {
                continue;
            }
            let centre = point_at(bx as f32 + 0.5, by as f32 + 0.5, d);
            // The surface normal from neighbouring depths, taking the nearer side each way.
            let neighbour = |dx: i64, dy: i64| -> Option<Vec3> {
                let nd = depth_at(i64::from(bx) + dx, i64::from(by) + dy);
                nd.is_finite().then(|| point_at(bx as f32 + 0.5 + dx as f32, by as f32 + 0.5 + dy as f32, nd))
            };
            let pick = |a: Option<Vec3>, b: Option<Vec3>, sign: f32| -> Option<Vec3> {
                match (a, b) {
                    (Some(a), Some(b)) => {
                        Some(if (a - centre).length_squared() < (b - centre).length_squared() { (a - centre) * sign } else { (centre - b) * sign })
                    }
                    (Some(a), None) => Some((a - centre) * sign),
                    (None, Some(b)) => Some((centre - b) * sign),
                    (None, None) => None,
                }
            };
            let (Some(along_x), Some(along_y)) = (pick(neighbour(1, 0), neighbour(-1, 0), 1.0), pick(neighbour(0, 1), neighbour(0, -1), 1.0)) else {
                continue;
            };
            let mut normal = along_y.cross(along_x).normalize_or_zero();
            if normal.dot(-centre) < 0.0 {
                normal = -normal;
            }
            if normal == Vec3::ZERO {
                continue;
            }
            let (tangent, bitangent) = normal.any_orthonormal_pair();
            // Turn the kernel per pixel (a 4 × 4 pattern) so its banding becomes noise the blur
            // removes.
            let angle = ((x % 4) * 4 + (y % 4)) as f32 * std::f32::consts::FRAC_PI_8;
            let (sin, cos) = angle.sin_cos();
            let mut occluded = 0.0;
            for sample in KERNEL {
                let (u, v) = (sample[0] * cos - sample[1] * sin, sample[0] * sin + sample[1] * cos);
                let offset = (tangent * u + bitangent * v + normal * sample[2]) * radius;
                let point = centre + offset;
                let clip = projection * point.extend(1.0);
                if clip.w <= 0.0 {
                    continue;
                }
                let ndc = clip.truncate() / clip.w;
                let sx = ((ndc.x + 1.0) * 0.5 * big_width as f32).floor() as i64;
                let sy = ((1.0 - ndc.y) * 0.5 * big_height as f32).floor() as i64;
                let surface = depth_at(sx, sy);
                let sample_depth = -point.z;
                if surface + 0.02 * radius < sample_depth {
                    // Only nearby surfaces occlude; a far wall in front does not.
                    let range = (radius / (sample_depth - surface).max(1e-4)).min(1.0);
                    occluded += range;
                }
            }
            if let Some(slot) = raw.get_mut(y as usize * width as usize + x as usize) {
                *slot = 1.0 - occluded / KERNEL.len() as f32;
            }
        }
    }
    // A 4 × 4 box blur hides the rotation pattern.
    let mut factor = raw.clone();
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            let mut sum = 0.0;
            let mut count = 0.0;
            for dy in -2..2 {
                for dx in -2..2 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= i64::from(width) || ny >= i64::from(height) {
                        continue;
                    }
                    if let Some(value) = raw.get(ny as usize * width as usize + nx as usize) {
                        sum += value;
                        count += 1.0;
                    }
                }
            }
            if let Some(slot) = factor.get_mut(y as usize * width as usize + x as usize)
                && count > 0.0
            {
                *slot = sum / count;
            }
        }
    }
    Occlusion { width, height, factor }
}
