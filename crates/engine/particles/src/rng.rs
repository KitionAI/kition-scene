//! Repeatable randomness: every number is a hash of the system's seed, the particle, and what
//! the number is for, so any particle at any frame comes out the same without a running state.

/// A uniform number in `[0, 1)` for `seed`, `index`, and `channel`.
pub fn unit(seed: u32, index: u32, channel: u32) -> f32 {
    let mut x = (u64::from(seed) << 32 | u64::from(index)) ^ (u64::from(channel).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    // SplitMix64's finaliser.
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    // The top 24 bits, exactly representable.
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// A uniformly random unit vector.
pub fn direction(seed: u32, index: u32, channel: u32) -> glam::Vec3 {
    let z = unit(seed, index, channel) * 2.0 - 1.0;
    let angle = unit(seed, index, channel + 1) * std::f32::consts::TAU;
    let ring = (1.0 - z * z).max(0.0).sqrt();
    glam::Vec3::new(ring * angle.cos(), ring * angle.sin(), z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_repeat_and_spread() {
        assert_eq!(unit(3, 17, 2), unit(3, 17, 2));
        assert_ne!(unit(3, 17, 2), unit(4, 17, 2));
        let mean = (0..10_000).map(|index| unit(1, index, 0)).sum::<f32>() / 10_000.0;
        assert!((mean - 0.5).abs() < 0.01, "{mean}");
        assert!((0..1000).all(|index| (0.0..1.0).contains(&unit(9, index, 5))));
        assert!((direction(1, 2, 3).length() - 1.0).abs() < 1e-5);
    }
}
