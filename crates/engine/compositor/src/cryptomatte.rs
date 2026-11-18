//! Cryptomatte (Friedman and Jones, "Fully Automatic ID Mattes with Support for Motion Blur and
//! Transparency", 2015, and its published specification): every pixel keeps the ids of the
//! objects, materials, or assets that cover it, most covering first, with how much each covers.
//! An id is a name's MurmurHash3 (x86, 32 bits, seed 0) read as a float, its exponent nudged off
//! zero and all ones so it is never subnormal, infinite, or not a number. The Cryptomatte node
//! picks mattes by name and adds up the coverage of the picked ids.

use std::collections::BTreeMap;

use crate::Rgba;

/// How many ranks a pixel keeps (the reference's default: six, in three RGBA pictures).
pub const DEPTH: usize = 6;

/// MurmurHash3's 32-bit x86 hash of `bytes` with seed 0.
pub fn murmur3(bytes: &[u8]) -> u32 {
    const C1: u32 = 0xcc9e_2d51;
    const C2: u32 = 0x1b87_3593;
    let mut hash = 0u32;
    let (blocks, tail) = bytes.as_chunks::<4>();
    for block in blocks {
        let mut k = u32::from_le_bytes(*block);
        k = k.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
        hash ^= k;
        hash = hash.rotate_left(13).wrapping_mul(5).wrapping_add(0xe654_6b64);
    }
    if !tail.is_empty() {
        let mut k = 0u32;
        for (index, byte) in tail.iter().enumerate() {
            k ^= u32::from(*byte) << (8 * index);
        }
        k = k.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
        hash ^= k;
    }
    hash ^= bytes.len() as u32;
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x85eb_ca6b);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(0xc2b2_ae35);
    hash ^ (hash >> 16)
}

/// A name's id: its hash as a float, never subnormal, infinite, or not a number.
pub fn id(name: &str) -> f32 {
    let mut hash = murmur3(name.as_bytes());
    let exponent = (hash >> 23) & 0xff;
    if exponent == 0 || exponent == 0xff {
        hash ^= 1 << 23;
    }
    f32::from_bits(hash)
}

/// A name as a manifest writes it: its hash in eight hexadecimal digits (before the exponent
/// is nudged, as the reference writes it).
pub fn hex(name: &str) -> String {
    format!("{:08x}", murmur3(name.as_bytes()))
}

/// One Cryptomatte layer: per pixel, up to `DEPTH` (id, coverage) pairs, most covering first,
/// and the names of the ids that occur (the manifest).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layer {
    pub ranks: Vec<Vec<(f32, f32)>>,
    pub names: BTreeMap<String, f32>,
}

impl Layer {
    /// A layer from each pixel's coverage by name (summing to at most one per pixel).
    pub fn from_coverage(coverage: &[BTreeMap<String, f32>]) -> Self {
        let mut names = BTreeMap::new();
        let ranks = coverage
            .iter()
            .map(|pixel| {
                let mut pairs: Vec<(f32, f32)> = pixel
                    .iter()
                    .filter(|(_, share)| **share > 0.0)
                    .map(|(name, share)| {
                        let id = id(name);
                        names.insert(name.clone(), id);
                        (id, *share)
                    })
                    .collect();
                pairs.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.to_bits().cmp(&b.0.to_bits())));
                pairs.truncate(DEPTH);
                pairs
            })
            .collect();
        Self { ranks, names }
    }

    /// The coverage of the ids in `picked` at each pixel.
    pub fn matte(&self, picked: &[f32]) -> Vec<f32> {
        self.ranks
            .iter()
            .map(|pairs| {
                pairs
                    .iter()
                    .filter(|(id, _)| picked.iter().any(|other| other.to_bits() == id.to_bits()))
                    .map(|(_, share)| *share)
                    .sum::<f32>()
                    .min(1.0)
            })
            .collect()
    }

    /// The layer as the reference's RGBA pictures (`CryptoObject00`, `01`, `02`): two ranks per
    /// picture, id then coverage.
    pub fn pictures(&self) -> Vec<Vec<Rgba>> {
        (0..DEPTH.div_ceil(2))
            .map(|picture| {
                self.ranks
                    .iter()
                    .map(|pairs| {
                        let rank = |index: usize| pairs.get(index).copied().unwrap_or((0.0, 0.0));
                        let (first, second) = (rank(picture * 2), rank(picture * 2 + 1));
                        [first.0, first.1, second.0, second.1]
                    })
                    .collect()
            })
            .collect()
    }

    /// The name with the most coverage at pixel `index`, if any covers it.
    pub fn name_at(&self, index: usize) -> Option<&str> {
        let (id, _) = *self.ranks.get(index)?.first()?;
        self.names.iter().find(|(_, other)| other.to_bits() == id.to_bits()).map(|(name, _)| name.as_str())
    }

    /// The ids `matte` names: names separated by commas, each its own name or, in angle
    /// brackets, an id written as a number (`<1.234e-5>`), as the reference's matte list has them.
    pub fn picked(matte: &str) -> Vec<f32> {
        matte
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| match name.strip_prefix('<').and_then(|rest| rest.strip_suffix('>')) {
                Some(number) => number.trim().parse::<f32>().unwrap_or(f32::NAN),
                None => id(name),
            })
            .filter(|id| id.is_finite())
            .collect()
    }

    /// The Pick output's colour at pixel `index`: the most covering id in red, and its bits
    /// from the ninth and from the seventeenth on, as fractions, in green and blue (as the
    /// reference shows it).
    pub fn pick(&self, index: usize) -> Rgba {
        let Some((id, _)) = self.ranks.get(index).and_then(|pairs| pairs.first()).copied() else { return [0.0, 0.0, 0.0, 1.0] };
        let bits = id.to_bits();
        let fraction = |shift: u32| (f64::from(bits << shift) / 4_294_967_296.0) as f32;
        [id, fraction(8), fraction(16), 1.0]
    }
}

/// A render's three Cryptomatte layers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layers {
    pub object: Layer,
    pub material: Layer,
    pub asset: Layer,
}

impl Layers {
    /// The layer a Cryptomatte node's `Layer` setting names.
    pub fn layer(&self, which: &str) -> &Layer {
        match which {
            "material" => &self.material,
            "asset" => &self.asset,
            _ => &self.object,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MurmurHash3's published check values (seed 0).
    #[test]
    fn the_hash_gives_the_published_values() {
        assert_eq!(murmur3(b""), 0);
        assert_eq!(murmur3(b"hello"), 0x248b_fa47);
        assert_eq!(murmur3(b"The quick brown fox jumps over the lazy dog"), 0x2e4f_f723);
    }

    #[test]
    fn ids_are_ordinary_floats() {
        for name in ["", "Cube", "Material", "a", "Suzanne.001"] {
            let id = id(name);
            assert!(id.is_finite() && (id == 0.0 || id.is_normal()), "{name}");
        }
    }

    #[test]
    fn a_matte_adds_the_picked_coverage() {
        let pixel = BTreeMap::from([("Cube".to_owned(), 0.25), ("Sphere".to_owned(), 0.75)]);
        let layer = Layer::from_coverage(&[pixel]);
        assert_eq!(layer.name_at(0), Some("Sphere"));
        assert_eq!(layer.matte(&Layer::picked("Cube")), vec![0.25]);
        assert_eq!(layer.matte(&Layer::picked("Cube, Sphere")), vec![1.0]);
        assert_eq!(layer.matte(&Layer::picked(&format!("<{:e}>", id("Sphere")))), vec![0.75]);
    }
}
