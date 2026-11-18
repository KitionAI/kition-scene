//! LZ4 block decompression, and the two wrappers binary USD layers use around it: chunked
//! "fast compression" (a chunk count, then LZ4 blocks) and integer compression (a common value,
//! two-bit codes, and variable-width deltas).

use kition_scene_model::SceneError;

use crate::usda::invalid;

/// The largest buffer a layer may decompress to (512 MiB).
pub const MAX_OUTPUT: usize = 512 * 1024 * 1024;

/// Decompresses one LZ4 block into `out` (appending), refusing anything that reads or writes
/// past its bounds or grows `out` beyond `limit` bytes.
pub fn block(input: &[u8], out: &mut Vec<u8>, limit: usize) -> Result<(), SceneError> {
    let start = out.len();
    let mut at = 0usize;
    let byte = |at: usize| input.get(at).copied().ok_or_else(|| invalid("an LZ4 block ends early"));
    let length = |at: &mut usize, mut value: usize| -> Result<usize, SceneError> {
        if value == 15 {
            loop {
                let more = byte(*at)?;
                *at += 1;
                value = value.checked_add(usize::from(more)).ok_or_else(|| invalid("an LZ4 length overflows"))?;
                if more != 255 {
                    break;
                }
            }
        }
        Ok(value)
    };
    while at < input.len() {
        let token = byte(at)?;
        at += 1;
        let literals = length(&mut at, usize::from(token >> 4))?;
        let end = at.checked_add(literals).ok_or_else(|| invalid("an LZ4 literal run overflows"))?;
        let run = input.get(at..end).ok_or_else(|| invalid("an LZ4 literal run passes the block's end"))?;
        if out.len() - start + run.len() > limit {
            return Err(invalid("an LZ4 block decompresses beyond its size"));
        }
        out.extend_from_slice(run);
        at = end;
        if at >= input.len() {
            break;
        }
        let offset = usize::from(byte(at)?) | (usize::from(byte(at + 1)?) << 8);
        at += 2;
        let count = length(&mut at, usize::from(token & 15))?.checked_add(4).ok_or_else(|| invalid("an LZ4 match overflows"))?;
        let written = out.len() - start;
        if offset == 0 || offset > written {
            return Err(invalid("an LZ4 match points before the block"));
        }
        if written + count > limit {
            return Err(invalid("an LZ4 block decompresses beyond its size"));
        }
        // Matches may overlap what they copy: copy byte by byte from `offset` back.
        let from = out.len() - offset;
        for step in 0..count {
            let value = out.get(from + step).copied().ok_or_else(|| invalid("an LZ4 match is damaged"))?;
            out.push(value);
        }
    }
    Ok(())
}

/// "Fast compression": a chunk count (0 for one block), then the blocks, each but a lone one led
/// by its compressed size as a 32-bit integer.
pub fn fast(input: &[u8], expected: usize) -> Result<Vec<u8>, SceneError> {
    if expected > MAX_OUTPUT {
        return Err(invalid("a compressed section is too large"));
    }
    let chunks = *input.first().ok_or_else(|| invalid("a compressed section is empty"))?;
    let mut out = Vec::with_capacity(expected);
    if chunks == 0 {
        block(input.get(1..).unwrap_or(&[]), &mut out, expected)?;
    } else {
        let mut at = 1usize;
        for _ in 0..chunks {
            let size = input.get(at..at + 4).ok_or_else(|| invalid("a compressed chunk ends early"))?;
            let size = i32::from_le_bytes(crate::lz4::le(size));
            let size = usize::try_from(size).map_err(|_| invalid("a compressed chunk has a negative size"))?;
            at += 4;
            let chunk = input
                .get(at..at.checked_add(size).ok_or_else(|| invalid("a compressed chunk overflows"))?)
                .ok_or_else(|| invalid("a compressed chunk ends early"))?;
            let remaining = expected.saturating_sub(out.len());
            block(chunk, &mut out, remaining)?;
            at += size;
        }
    }
    if out.len() != expected {
        return Err(invalid("a compressed section decompresses to the wrong size"));
    }
    Ok(out)
}

/// Integer compression of `count` 32-bit integers (signed deltas from the previous value).
pub fn ints32(input: &[u8], count: usize) -> Result<Vec<i32>, SceneError> {
    let working = 4usize
        .checked_add(count.checked_mul(2).ok_or_else(|| invalid("too many integers"))?.div_ceil(8))
        .and_then(|size| size.checked_add(count.checked_mul(4)?))
        .ok_or_else(|| invalid("too many integers"))?;
    let encoded = fast_prefix(input, working)?;
    decode(&encoded, count, 4, |bytes| i64::from(i32::from_le_bytes(le(bytes))))
        .and_then(|values| values.into_iter().map(|value| i32::try_from(value).map_err(|_| invalid("an integer overflows"))).collect())
}

/// Integer compression of `count` 64-bit integers.
pub fn ints64(input: &[u8], count: usize) -> Result<Vec<i64>, SceneError> {
    let working = 8usize
        .checked_add(count.checked_mul(2).ok_or_else(|| invalid("too many integers"))?.div_ceil(8))
        .and_then(|size| size.checked_add(count.checked_mul(8)?))
        .ok_or_else(|| invalid("too many integers"))?;
    let encoded = fast_prefix(input, working)?;
    decode(&encoded, count, 8, |bytes| i64::from_le_bytes(le(bytes)))
}

/// Decompresses into at most `working` bytes (the encoded integers are shorter than the
/// worst case the size is computed from).
fn fast_prefix(input: &[u8], working: usize) -> Result<Vec<u8>, SceneError> {
    if working > MAX_OUTPUT {
        return Err(invalid("a compressed integer array is too large"));
    }
    let chunks = *input.first().ok_or_else(|| invalid("a compressed integer array is empty"))?;
    let mut out = Vec::new();
    if chunks == 0 {
        block(input.get(1..).unwrap_or(&[]), &mut out, working)?;
    } else {
        let mut at = 1usize;
        for _ in 0..chunks {
            let size = input.get(at..at + 4).ok_or_else(|| invalid("a compressed chunk ends early"))?;
            let size = usize::try_from(i32::from_le_bytes(crate::lz4::le(size))).map_err(|_| invalid("a compressed chunk has a negative size"))?;
            at += 4;
            let chunk = input
                .get(at..at.checked_add(size).ok_or_else(|| invalid("a compressed chunk overflows"))?)
                .ok_or_else(|| invalid("a compressed chunk ends early"))?;
            let remaining = working.saturating_sub(out.len());
            block(chunk, &mut out, remaining)?;
            at += size;
        }
    }
    Ok(out)
}

/// The first `N` bytes of `bytes` (zero-filled when it is shorter), for little-endian reads.
pub(crate) fn le<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    for (slot, byte) in out.iter_mut().zip(bytes) {
        *slot = *byte;
    }
    out
}

/// The encoded form: a common value of `width` bytes, `count` two-bit codes (four per byte,
/// lowest first), then the deltas the codes size: 0 the common value, 1 a quarter, 2 a half,
/// 3 a full width integer.
fn decode(data: &[u8], count: usize, width: usize, common_of: impl Fn(&[u8]) -> i64) -> Result<Vec<i64>, SceneError> {
    let common = common_of(data.get(..width).ok_or_else(|| invalid("compressed integers end early"))?);
    let codes_start = width;
    let codes_len = (count * 2).div_ceil(8);
    let mut at = codes_start + codes_len;
    let mut previous = 0i64;
    let mut out = Vec::with_capacity(count);
    let read = |at: &mut usize, size: usize| -> Result<i64, SceneError> {
        let bytes = data.get(*at..*at + size).ok_or_else(|| invalid("compressed integers end early"))?;
        *at += size;
        Ok(match size {
            1 => i64::from(i8::from_le_bytes(le(bytes))),
            2 => i64::from(i16::from_le_bytes(crate::lz4::le(bytes))),
            4 => i64::from(i32::from_le_bytes(crate::lz4::le(bytes))),
            _ => i64::from_le_bytes(le(bytes)),
        })
    };
    for index in 0..count {
        let code_byte = *data.get(codes_start + index / 4).ok_or_else(|| invalid("compressed integers end early"))?;
        let code = (code_byte >> (2 * (index % 4))) & 3;
        let delta = match code {
            0 => common,
            1 => read(&mut at, width / 4)?,
            2 => read(&mut at, width / 2)?,
            _ => read(&mut at, width)?,
        };
        previous = previous.wrapping_add(delta);
        out.push(previous);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_literals_and_overlapping_matches() {
        // "abc" then a match of offset 3, length 9 → "abcabcabcabc".
        let input = [0x35, b'a', b'b', b'c', 3, 0];
        let mut out = Vec::new();
        block(&input, &mut out, 100).unwrap();
        assert_eq!(out, b"abcabcabcabc");
    }

    #[test]
    fn refuses_matches_before_the_start_and_overflow() {
        let mut out = Vec::new();
        assert!(block(&[0x10, b'a', 9, 0], &mut out, 100).is_err());
        let mut out = Vec::new();
        assert!(block(&[0x3f, b'a', b'b', b'c', 1, 0, 255, 255, 255, 10], &mut out, 50).is_err());
        assert!(fast(&[], 4).is_err());
    }

    #[test]
    fn integer_codes_follow_deltas() {
        // common 1; codes: 0 (common), 1 (i8 5), 3 (i32 -10) → 1, 6, -4.
        let mut encoded = 1i32.to_le_bytes().to_vec();
        encoded.push(0b11_01_00);
        encoded.push(5);
        encoded.extend((-10i32).to_le_bytes());
        let values = decode(&encoded, 3, 4, |bytes| i64::from(i32::from_le_bytes(le(bytes)))).unwrap();
        assert_eq!(values, vec![1, 6, -4]);
    }
}
