//! USD integer tables: a common delta, packed two-bit widths, then signed differences.

use kition_scene_model::SceneError;

use super::index;
use crate::usda::invalid;

pub(super) fn integers(values: impl ExactSizeIterator<Item = i32> + Clone) -> Result<Vec<u8>, SceneError> {
    index(values.len())?;
    // A single-byte delta is common in table indices. Choose its mode without a hash
    // table; ties choose the lower signed value, independent of map iteration order.
    let mut counts = [0usize; 256];
    let mut previous = 0i32;
    for value in values.clone() {
        let delta = value.checked_sub(previous).ok_or_else(|| invalid("a binary table delta exceeds int32"))?;
        if let Ok(byte) = i8::try_from(delta)
            && let Some(count) = counts.get_mut((i16::from(byte) + 128) as usize)
        {
            *count += 1;
        }
        previous = value;
    }
    let (mut common, mut most) = (0i32, 0usize);
    for (slot, count) in counts.into_iter().enumerate() {
        if count > most {
            common = slot as i32 - 128;
            most = count;
        }
    }
    let codes = values.len().div_ceil(4);
    let mut bytes = Vec::with_capacity(4 + codes + values.len());
    bytes.extend_from_slice(&common.to_le_bytes());
    bytes.resize(4 + codes, 0);
    previous = 0;
    for (position, value) in values.enumerate() {
        let delta = value.checked_sub(previous).ok_or_else(|| invalid("a binary table delta exceeds int32"))?;
        let code = if delta == common {
            0
        } else if let Ok(byte) = i8::try_from(delta) {
            bytes.push(byte as u8);
            1
        } else if let Ok(short) = i16::try_from(delta) {
            bytes.extend_from_slice(&short.to_le_bytes());
            2
        } else {
            bytes.extend_from_slice(&delta.to_le_bytes());
            3
        };
        if let Some(slot) = bytes.get_mut(4 + position / 4) {
            *slot |= code << (2 * (position % 4));
        }
        previous = value;
    }
    Ok(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::{lz4, usdc::write::Writer};

    fn round_trip(values: &[i32]) -> Vec<u8> {
        let encoded = integers(values.iter().copied()).unwrap();
        let mut writer = Writer::default();
        writer.compressed(&encoded).unwrap();
        assert_eq!(lz4::ints32(&writer.bytes[8..], values.len()).unwrap(), values);
        encoded
    }

    #[test]
    fn integer_tables_use_all_signed_widths_and_stable_common_deltas() {
        let deltas = [1, 1, 1, 0, 127, -128, 128, -129, 32767, -32768, 32768, -32769];
        let mut value = 0;
        let values: Vec<_> = deltas
            .into_iter()
            .map(|delta| {
                value += delta;
                value
            })
            .collect();
        let encoded = round_trip(&values);
        assert_eq!(&encoded[..4], &1i32.to_le_bytes());
        assert_eq!(&encoded[4..7], &[0b01000000, 0b10100101, 0b11111010]);
        assert_eq!(&round_trip(&[1, 3, 4, 6])[..4], &1i32.to_le_bytes());
        round_trip(&[]);
        round_trip(&[0]);
        round_trip(&[i32::MAX, 0, i32::MIN]);
        assert!(integers([i32::MAX, i32::MIN].into_iter()).is_err());
        assert!(integers(std::iter::repeat_n(0, super::super::MAX_ENTRIES + 1)).is_err());
    }

    #[test]
    fn lz4_tables_compress_matches_and_round_trip_incompressible_bytes() {
        let mut random = 1u32;
        let noise: Vec<_> = (0..16_384)
            .map(|_| {
                random ^= random << 13;
                random ^= random >> 17;
                random ^= random << 5;
                random as u8
            })
            .collect();
        for input in [vec![], vec![7], noise, b"Repeated USD field names and paths\0".repeat(1024)] {
            let mut writer = Writer::default();
            writer.compressed(&input).unwrap();
            assert_eq!(lz4::fast(&writer.bytes[8..], input.len()).unwrap(), input);
            if input.len() > 16_384 {
                assert!(writer.bytes.len() < input.len() / 4);
            }
        }
        let indices: Vec<_> = (0..10_000).collect();
        let encoded = round_trip(&indices);
        assert!(encoded.len() < indices.len());
    }

    #[test]
    fn literal_blocks_bound_large_table_compression_on_32_bit_targets() {
        let input = vec![b't'; 40 * 1024 * 1024];
        assert!((input.len() as u32).checked_mul(110).is_none());
        let mut writer = Writer::default();
        writer.compressed(&input).unwrap();
        assert!(writer.bytes.len() > input.len());
        assert_eq!(lz4::fast(&writer.bytes[8..], input.len()).unwrap(), input);
    }
}
