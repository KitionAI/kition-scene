//! Deterministic random numbers from an element's id and a seed: the same id and seed give the
//! same value on every machine and in every run (so a scattered forest stays where it was).

/// A 32-bit mix of three values (a finaliser in the style of murmur3's).
pub fn hash(a: u32, b: u32, c: u32) -> u32 {
    let mut value = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77) ^ c.wrapping_mul(0xC2B2_AE3D);
    value ^= value >> 16;
    value = value.wrapping_mul(0x7FEB_352D);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846C_A68B);
    value ^= value >> 16;
    value
}

/// A number in [0, 1) for element `id`, `seed`, and `channel` (a vector's axis).
pub fn unit(id: i32, seed: i32, channel: u32) -> f32 {
    let bits = hash(id as u32, seed as u32, channel);
    // 24 bits make every value exact in an f32.
    (bits >> 8) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_spread_and_stable() {
        let values: Vec<f32> = (0..10_000).map(|id| unit(id, 0, 0)).collect();
        assert!(values.iter().all(|value| (0.0..1.0).contains(value)));
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
        assert_eq!(unit(7, 3, 1), unit(7, 3, 1));
        assert_ne!(unit(7, 3, 1), unit(7, 4, 1));
    }
}
