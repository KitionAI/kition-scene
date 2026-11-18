//! Standard base64 (RFC 4648, with padding), for glTF data URIs.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes bytes.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk.first().copied().unwrap_or(0);
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let triple = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        let symbol = |shift: u32| ALPHABET.get(((triple >> shift) & 63) as usize).map_or('A', |byte| char::from(*byte));
        out.push(symbol(18));
        out.push(symbol(12));
        out.push(if chunk.len() > 1 { symbol(6) } else { '=' });
        out.push(if chunk.len() > 2 { symbol(0) } else { '=' });
    }
    out
}

fn value(byte: u8) -> Option<u32> {
    match byte {
        b'A'..=b'Z' => Some(u32::from(byte - b'A')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

/// Decodes base64 (padding optional, whitespace ignored); `None` for a bad symbol.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let symbols: Vec<u8> = text.bytes().filter(|byte| !byte.is_ascii_whitespace()).collect();
    let trimmed: &[u8] = {
        let end = symbols.iter().rposition(|byte| *byte != b'=').map_or(0, |at| at + 1);
        symbols.get(..end)?
    };
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    for chunk in trimmed.chunks(4) {
        let mut triple = 0u32;
        for (slot, byte) in chunk.iter().enumerate() {
            triple |= value(*byte)? << (18 - 6 * slot as u32);
        }
        match chunk.len() {
            4 => out.extend([(triple >> 16) as u8, (triple >> 8) as u8, triple as u8]),
            3 => out.extend([(triple >> 16) as u8, (triple >> 8) as u8]),
            2 => out.push((triple >> 16) as u8),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for length in 0..20u8 {
            let bytes: Vec<u8> = (0..length).map(|value| value.wrapping_mul(37)).collect();
            assert_eq!(decode(&encode(&bytes)).unwrap(), bytes);
        }
        assert_eq!(encode(b"Man"), "TWFu");
        assert_eq!(encode(b"Ma"), "TWE=");
        assert!(decode("T$Fu").is_none());
    }
}
